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
        let text = interrupt_result_text(cancels_turn(&steps));
        let pending = turns.results.join(format!(".{entry}"));
        if write_file(&pending, text).is_ok() {
            fs::rename(&pending, turns.results.join(&entry)).ok();
        }
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
        fs::remove_file(&turns.markers.waiting).ok();
        turns.heartbeat.set_running(true);
        turns.reported.store(false, Ordering::SeqCst);
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
                _ = stop => return,
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
        let outcome = turn_outcome(
            reason.as_deref().map_err(|e| e.message.as_str()),
            turns.reported.load(Ordering::SeqCst),
            text == REPORT_REMINDER,
            turns.closing.load(Ordering::SeqCst),
            interrupted,
        );
        match outcome {
            TurnOutcome::Remind => {
                queue.accept(Inbound::Prompt(REPORT_REMINDER.to_string()));
            }
            TurnOutcome::Unreported => {
                write_file(&turns.markers.unreported, "").ok();
                write_file(&turns.markers.waiting, "").ok();
            }
            TurnOutcome::Cancelled => turns.permissions.cancel_all(),
            TurnOutcome::Problem(problem) => post_prompt_failure(turns, &problem),
            TurnOutcome::Stopped(reason) => {
                post_prompt_failure(turns, &stop_problem(&turns.agent, &reason));
            }
            TurnOutcome::Reported | TurnOutcome::Closed => {}
        }
    }
}

fn post_prompt_failure(turns: &Turns, problem: &str) {
    let at = chrono::Utc::now().to_rfc3339();
    let report = prompt_failure_report(&turns.agent, &turns.worker_id, &at, problem);
    turns.inbox.post(&report, 0).ok();
}
