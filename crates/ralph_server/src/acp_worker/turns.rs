use super::cancel::Cancellation;
use super::*;
use crate::worker_mcp::remove_if_present;
use futures::select_biased;
use ralph_core::worker::{turn_end_activity, ActivityState};

pub(super) struct Turns {
    pub(super) agent: AcpAgent,
    pub(super) permissions: Arc<Permissions>,
    pub(super) session: SessionId,
    pub(super) worker_id: String,
    pub(super) markers: WorkerMarkers,
    pub(super) results: PathBuf,
    pub(super) inbox: Inbox,
    pub(super) closing: Arc<AtomicBool>,
    pub(super) changes: Option<WorkerChanges>,
    pub(super) heartbeat: Arc<Heartbeat>,
    pub(super) events: Events,
    pub(super) generation: Arc<AtomicU64>,
    pub(super) group: u32,
}

impl Turns {
    fn inbox_dir(&self) -> Option<PathBuf> {
        (self.agent == AGY_AGENT)
            .then(|| {
                self.results
                    .parent()?
                    .parent()
                    .map(|store| journal::directory(store, &self.worker_id))
            })
            .flatten()
    }
}

fn run_steps(connection: &ConnectionTo<Agent>, turns: &Turns, steps: Vec<Step>) {
    for step in steps {
        match step {
            Step::CancelPermissions => turns.permissions.cancel_all(),
            Step::CancelSession => {
                if let Some(AcpCall::Cancel) = encode(WorkerCommand::Interrupt(String::new())) {
                    connection
                        .send_notification(CancelNotification::new(turns.session.clone()))
                        .ok();
                }
            }
        }
    }
}

fn accept_inbound(
    connection: &ConnectionTo<Agent>,
    turns: &Turns,
    queue: &mut PromptQueue,
    item: Inbound,
    cancellation: &mut Cancellation,
) {
    if let Inbound::TrackedInterrupt(text, entry, receipt, target) = item {
        if turns.closing.load(Ordering::SeqCst) {
            write_interrupt_result(
                &turns.results,
                &entry,
                ralph_core::worker::InterruptOutcome::Gone,
            );
            return;
        }
        queue.accept_ordered_interrupt(text, receipt);
        let outcome = cancellation.request(
            entry.clone(),
            target,
            turns.generation.load(Ordering::SeqCst),
            || {
                let sent = connection
                    .send_notification(CancelNotification::new(turns.session.clone()))
                    .is_ok();
                if sent {
                    turns.permissions.cancel_all();
                }
                sent
            },
        );
        if let Some(outcome) = outcome {
            write_interrupt_result(&turns.results, &entry, outcome);
        }
        return;
    }
    let entry = match &item {
        Inbound::Interrupt(_, entry) => Some(entry.clone()),
        Inbound::Prompt(_)
        | Inbound::TrackedPrompt(_, _)
        | Inbound::TrackedInterrupt(_, _, _, _) => None,
    };
    let steps = queue.accept(item);
    if let Some(entry) = entry {
        write_interrupt_result(&turns.results, &entry, cancels_turn(&steps).into());
    }
    run_steps(connection, turns, steps);
}

pub(super) async fn run_turns(
    connection: &ConnectionTo<Agent>,
    turns: &Turns,
    inbound: queue::UnboundedReceiver<Inbound>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut inbound = inbound.fuse();
    let mut queue = PromptQueue::default();
    let mut stopping = false;
    let mut cancellation = Cancellation::default();
    loop {
        let Some((text, receipt)) = queue.start_tracked() else {
            match futures::future::select(inbound.next(), &mut stop).await {
                Either::Left((Some(item), _)) => {
                    accept_inbound(connection, turns, &mut queue, item, &mut cancellation);
                    continue;
                }
                _ => {
                    while let Some(Some(item)) = inbound.next().now_or_never() {
                        if let Inbound::TrackedInterrupt(_, entry, _, _) = item {
                            write_interrupt_result(
                                &turns.results,
                                &entry,
                                ralph_core::worker::InterruptOutcome::Gone,
                            );
                        }
                    }
                    return;
                }
            }
        };
        push_event(&turns.events, WorkerEvent::TurnStarted);
        turns.heartbeat.set_state(ActivityState::Running);
        if let Err(error) = remove_if_present(&turns.markers.reported) {
            queue.finish();
            let message = format!("could not clear the report receipt: {error}");
            turns.heartbeat.set_state(ActivityState::Failed);
            push_event(
                &turns.events,
                WorkerEvent::TurnEnded(TurnEnd::Failed {
                    message: message.clone(),
                    transient: false,
                }),
            );
            post_prompt_failure(turns, &message);
            continue;
        }
        let mut active_token = None;
        if turns.agent == AGY_AGENT {
            let generation = turns.generation.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(store) = turns
                .inbox_dir()
                .and_then(|dir| dir.parent()?.parent().map(Path::to_path_buf))
            {
                match journal::publish_turn(&store, &turns.worker_id, generation, turns.group) {
                    Ok(token) => active_token = Some((store, token)),
                    Err(error) => {
                        turns.generation.fetch_add(1, Ordering::SeqCst);
                        queue.finish();
                        let message = format!("could not publish agy turn: {error}");
                        turns.heartbeat.set_state(ActivityState::Failed);
                        push_event(
                            &turns.events,
                            WorkerEvent::TurnEnded(TurnEnd::Failed {
                                message: message.clone(),
                                transient: false,
                            }),
                        );
                        post_prompt_failure(turns, &message);
                        continue;
                    }
                }
            }
        }
        let Some(AcpCall::Prompt(prompt)) = encode(WorkerCommand::Prompt(text.clone())) else {
            return;
        };
        let mut sent = Box::pin(
            connection
                .send_request(PromptRequest::new(
                    turns.session.clone(),
                    vec![prompt.into()],
                ))
                .block_task(),
        )
        .fuse();
        let result = loop {
            select_biased! {
                result = sent => break result,
                _ = stop => {
                    stopping = true;
                    if turns.agent != AGY_AGENT || !cancellation.pending() {
                        run_steps(connection, turns, vec![Step::CancelSession]);
                    }
                }
                item = inbound.select_next_some() => {
                    accept_inbound(connection, turns, &mut queue, item, &mut cancellation);
                }
            }
        };
        if turns.agent == AGY_AGENT {
            turns.generation.fetch_add(1, Ordering::SeqCst);
            if let Some((store, token)) = active_token {
                journal::finish_turn(&store, &turns.worker_id, &token);
            }
        }
        let queued_interrupt = queue.finish();
        let reason = result.as_ref().map(|done| {
            serde_json::to_value(done.stop_reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default()
        });
        let stop = reason.as_deref().map_err(|e| e.message.as_str());
        let interrupted = if turns.agent == AGY_AGENT {
            stop == Ok("cancelled")
        } else {
            queued_interrupt
        };
        for (entry, outcome) in cancellation.finish(stop, stopping) {
            write_interrupt_result(&turns.results, &entry, outcome);
        }
        if result.is_ok() {
            if let Some(store) = turns.inbox_dir() {
                journal::finish(&store, receipt.as_deref());
            }
        }
        let end = turn_end(&result, interrupted);
        turns.heartbeat.set_state(turn_end_activity(&end));
        if let Some(changes) = turns
            .changes
            .as_ref()
            .filter(|_| !turns.closing.load(Ordering::SeqCst))
        {
            changes.snapshot_turn_or_warn();
        }
        push_event(&turns.events, WorkerEvent::TurnEnded(end));
        let outcome = receipt_outcome(
            &turns.markers.reported,
            stop,
            text == REPORT_REMINDER,
            turns.closing.load(Ordering::SeqCst),
            interrupted,
        );
        if sweeps_permissions(stop, &outcome) {
            turns.permissions.cancel_all();
        }
        match outcome {
            TurnOutcome::Remind => queue.remind(),
            TurnOutcome::Unreported => {
                write_file(&turns.markers.unreported, "").ok();
                write_file(&turns.markers.waiting, "").ok();
            }
            TurnOutcome::Problem(problem) => post_prompt_failure(turns, &problem),
            TurnOutcome::Stopped(reason) => {
                post_prompt_failure(turns, &stop_problem(turns.agent.name, &reason));
            }
            TurnOutcome::Reported | TurnOutcome::Cancelled | TurnOutcome::Closed => {}
        }
        if stopping {
            while let Some(Some(item)) = inbound.next().now_or_never() {
                if let Inbound::TrackedInterrupt(_, entry, _, _) = item {
                    write_interrupt_result(
                        &turns.results,
                        &entry,
                        ralph_core::worker::InterruptOutcome::Gone,
                    );
                }
            }
            return;
        }
    }
}

fn turn_end<T>(result: &Result<T, agent_client_protocol::Error>, interrupted: bool) -> TurnEnd {
    match result {
        Err(e) => TurnEnd::Failed {
            message: e.message.clone(),
            transient: false,
        },
        Ok(_) if interrupted => TurnEnd::Interrupted,
        Ok(_) => TurnEnd::Succeeded,
    }
}

fn post_prompt_failure(turns: &Turns, problem: &str) {
    let at = chrono::Utc::now().to_rfc3339();
    let report = prompt_failure_report(turns.agent.name, &turns.worker_id, &at, problem);
    turns.inbox.post(&report, 0).ok();
}

fn receipt_outcome(
    receipt: &Path,
    stop: Result<&str, &str>,
    reminded: bool,
    closing: bool,
    interrupted: bool,
) -> TurnOutcome {
    turn_outcome(stop, receipt.exists(), reminded, closing, interrupted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_prompt_error_is_not_reported_as_cancelled() {
        let result: Result<(), agent_client_protocol::Error> =
            Err(agent_client_protocol::Error::internal_error());
        assert!(matches!(turn_end(&result, true), TurnEnd::Failed { .. }));
    }

    #[test]
    fn rejected_report_requires_a_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let markers = WorkerMarkers::new(dir.path(), "A");
        let inbox =
            Inbox::new(dir.path().join("inbox")).requiring_owner(Some(dir.path().join("agy/A")));
        let call = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"report\",\"arguments\":{\"status\":\"done\",\"text\":\"hello\"}}}\n";
        let mut output = Vec::new();
        crate::worker_mcp::run_stdio_server("A", &inbox, None, &call[..], &mut output).unwrap();
        let reply: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            receipt_outcome(&markers.reported, Ok("end_turn"), false, false, false),
            TurnOutcome::Remind
        );
    }

    #[test]
    fn report_receipt_belongs_to_the_current_worker_and_turn() {
        let dir = tempfile::tempdir().unwrap();
        let a = WorkerMarkers::new(dir.path(), "A");
        let b = WorkerMarkers::new(dir.path(), "B");
        let inbox = Inbox::new(dir.path().join("inbox"));
        let call = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"report\",\"arguments\":{\"status\":\"done\",\"text\":\"hello\"}}}\n";
        let mut output = Vec::new();
        crate::worker_mcp::run_stdio_server("A", &inbox, None, &call[..], &mut output).unwrap();
        assert_eq!(
            receipt_outcome(&a.reported, Ok("end_turn"), false, false, false),
            TurnOutcome::Reported
        );
        assert_eq!(
            receipt_outcome(&b.reported, Ok("end_turn"), false, false, false),
            TurnOutcome::Remind
        );
        remove_if_present(&a.reported).unwrap();
        assert_eq!(
            receipt_outcome(&a.reported, Ok("end_turn"), false, false, false),
            TurnOutcome::Remind
        );
        remove_if_present(&a.reported).unwrap();
    }

    #[test]
    fn receipt_reset_rejects_unremovable_stale_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = dir.path().join("receipt");
        fs::create_dir(&receipt).unwrap();
        assert!(remove_if_present(&receipt).is_err());
    }
}
