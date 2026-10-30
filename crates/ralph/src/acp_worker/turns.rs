use super::*;

pub(super) struct Turns {
    pub(super) agent: AcpAgent,
    pub(super) permissions: Arc<Permissions>,
    pub(super) session: SessionId,
    pub(super) worker_id: String,
    pub(super) markers: WorkerMarkers,
    pub(super) results: PathBuf,
    pub(super) inbox: Inbox,
    pub(super) reported: Arc<AtomicBool>,
    pub(super) closing: Arc<AtomicBool>,
    pub(super) changes: Option<WorkerChanges>,
    pub(super) heartbeat: Arc<Heartbeat>,
    pub(super) events: Events,
    pub(super) updated: Arc<AtomicBool>,
    pub(super) pending: Option<(PathBuf, SessionRecord)>,
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
) {
    let entry = match &item {
        Inbound::Interrupt(_, entry) => Some(entry.clone()),
        Inbound::Prompt(_) => None,
    };
    let steps = queue.accept(item);
    if let Some(entry) = entry {
        write_interrupt_result(&turns.results, &entry, cancels_turn(&steps));
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
    let mut pending = turns.pending.clone();
    let mut stopping = false;
    loop {
        let Some(text) = queue.start() else {
            match futures::future::select(inbound.next(), &mut stop).await {
                Either::Left((Some(item), _)) => {
                    accept_inbound(connection, turns, &mut queue, item);
                    continue;
                }
                _ => return,
            }
        };
        push_event(&turns.events, WorkerEvent::TurnStarted);
        turns.heartbeat.set_running(true);
        turns.reported.store(false, Ordering::SeqCst);
        turns.updated.store(false, Ordering::SeqCst);
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
            select! {
                result = sent => break result,
                _ = stop => {
                    stopping = true;
                    run_steps(connection, turns, vec![Step::CancelSession]);
                }
                item = inbound.select_next_some() => {
                    accept_inbound(connection, turns, &mut queue, item);
                }
            }
        };
        turns.heartbeat.set_running(false);
        let interrupted = queue.finish();
        if let Some(changes) = turns
            .changes
            .as_ref()
            .filter(|_| !turns.closing.load(Ordering::SeqCst))
        {
            changes.snapshot_turn_or_warn();
        }
        let reason = result.as_ref().map(|done| {
            serde_json::to_value(done.stop_reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default()
        });
        if session_saved(
            reason.as_deref().map_err(|e| e.message.as_str()),
            turns.updated.load(Ordering::SeqCst),
        ) {
            if let Some((path, record)) = pending.take() {
                save_record(turns, &path, &record);
            }
        }
        push_event(
            &turns.events,
            WorkerEvent::TurnEnded(turn_end(&result, interrupted)),
        );
        let stop = reason.as_deref().map_err(|e| e.message.as_str());
        let outcome = turn_outcome(
            stop,
            turns.reported.load(Ordering::SeqCst),
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
            return;
        }
    }
}

fn save_record(turns: &Turns, path: &Path, record: &SessionRecord) {
    if let Err(e) = write_file(path, &record.render()) {
        warn(format!(
            "could not write the {} session record: {e}",
            turns.agent.name
        ));
    }
}

fn turn_end<T>(result: &Result<T, agent_client_protocol::Error>, interrupted: bool) -> TurnEnd {
    match result {
        _ if interrupted => TurnEnd::Interrupted,
        Ok(_) => TurnEnd::Succeeded,
        Err(e) => TurnEnd::Failed {
            message: e.message.clone(),
            transient: false,
        },
    }
}

fn post_prompt_failure(turns: &Turns, problem: &str) {
    let at = chrono::Utc::now().to_rfc3339();
    let report = prompt_failure_report(turns.agent.name, &turns.worker_id, &at, problem);
    turns.inbox.post(&report, 0).ok();
}
