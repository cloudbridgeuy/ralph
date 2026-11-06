use super::{agent_version_warning, SHUTDOWN_GRACE};
use crate::acp_log::{drain_stderr, CappedLog};
use crate::cursor_owner::{claim_owner, report_start_warning, write_file, write_interrupt_result};
use crate::cursor_sweep::sweep_worker_server;
use crate::cursor_usage::{record_cursor_usage, CursorTurn};
use crate::subprocess::unregister_group;
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::watches::locked;
use crate::worker_changes::WorkerChanges;
use crate::worker_events::{apply_effect, push_event, Events};
use crate::worker_mcp::Inbox;
use crate::workers::headless::Heartbeat;
use ralph_core::snapshot::WorkerSite;
use ralph_core::worker::{
    acp_mcp_server, bind_started_turn, claude_mcp_config, cursor_mcp_json, decode_print_line,
    exit_outcome, inbox_env, next_chat, print_args, print_model, prompt_failure_report, reduce,
    task_prompt, turn_end_activity, turn_end_for, Action, ActivityState, ExitOutcome, HandoffFiles,
    HarnessId, HeadlessState, ModelUsage, PermissionMode, PrintArgs, PrintEvent, Scope,
    SessionRecord, SpawnRequest, ThreadSignal, TurnEnd, Turns, WorkerEvent, WorkerMarkers,
    CURSOR_AGENT, SERVER_NAME,
};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const INTERRUPT_GRACE: Duration = Duration::from_secs(3);
const NO_SIGINT: u64 = u64::MAX;
const RESULT_GRACE: Duration = Duration::from_millis(1500);

#[derive(Debug, PartialEq)]
enum ReadPlan {
    Block,
    Wait(Duration),
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TurnProc {
    Idle,
    Starting { interrupt: bool },
    Running { pgid: u32, turn: u64 },
    Grace { pgid: u32, turn: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Group {
    pgid: u32,
    turn: u64,
}

#[derive(Debug, PartialEq)]
enum InterruptPlan {
    Send(Group),
    Hold,
    Drop,
}

fn sigint_hit_turn(signalled: u64, turn: u64) -> bool {
    signalled == turn
}

fn begin_start() -> TurnProc {
    TurnProc::Starting { interrupt: false }
}

fn on_interrupt(state: TurnProc) -> (TurnProc, InterruptPlan) {
    match state {
        TurnProc::Starting { .. } => (TurnProc::Starting { interrupt: true }, InterruptPlan::Hold),
        TurnProc::Running { pgid, turn } => (state, InterruptPlan::Send(Group { pgid, turn })),
        TurnProc::Grace { .. } | TurnProc::Idle => (state, InterruptPlan::Drop),
    }
}

fn on_spawned(state: TurnProc, group: Group) -> (TurnProc, Option<Group>) {
    let held = matches!(state, TurnProc::Starting { interrupt: true });
    let Group { pgid, turn } = group;
    (TurnProc::Running { pgid, turn }, held.then_some(group))
}

fn on_result(state: TurnProc) -> TurnProc {
    match state {
        TurnProc::Running { pgid, turn } => TurnProc::Grace { pgid, turn },
        other => other,
    }
}

fn on_release() -> TurnProc {
    TurnProc::Idle
}

fn live_group(state: TurnProc) -> Option<u32> {
    match state {
        TurnProc::Running { pgid, .. } | TurnProc::Grace { pgid, .. } => Some(pgid),
        TurnProc::Idle | TurnProc::Starting { .. } => None,
    }
}

fn escalation_target(state: TurnProc, group: Group) -> Option<u32> {
    match state {
        TurnProc::Running { pgid, turn } if Group { pgid, turn } == group => Some(pgid),
        _ => None,
    }
}

fn kill_after_term(state: TurnProc, group: Group) -> bool {
    escalation_target(state, group).is_some()
}

fn read_plan(since_result: Option<Duration>, grace: Duration) -> ReadPlan {
    match since_result {
        None => ReadPlan::Block,
        Some(elapsed) if elapsed < grace => ReadPlan::Wait(grace - elapsed),
        Some(_) => ReadPlan::Stop,
    }
}

type TurnRun = (ExitOutcome, Option<TurnEnd>, Option<ModelUsage>);

fn closed_turn() -> TurnRun {
    (ExitOutcome::Interrupted, None, None)
}

struct Run {
    store: PathBuf,
    worktree: PathBuf,
    model: String,
    session: PathBuf,
    record: SessionRecord,
}

struct Shared {
    id: String,
    run: Run,
    turns: Mutex<Turns>,
    events: Events,
    heartbeat: Arc<Heartbeat>,
    markers: WorkerMarkers,
    inbox: Inbox,
    results: PathBuf,
    chat: Mutex<Option<String>>,
    changes: Option<WorkerChanges>,
    snapshot: Mutex<Option<u64>>,
    proc: Mutex<TurnProc>,
    turn_seq: AtomicU64,
    spawns: AtomicU64,
    sigint_turn: AtomicU64,
    closing: AtomicBool,
}

impl Shared {
    fn drive(self: &Arc<Self>, step: impl FnOnce(&mut Turns) -> Vec<Action>) {
        let mut turns = locked(&self.turns);
        let actions = step(&mut turns);
        let seq = self.turn_seq.fetch_add(1, Ordering::SeqCst);
        let actions = bind_started_turn(&mut turns, actions, seq);
        drop(turns);
        self.perform(actions);
    }

    fn perform(self: &Arc<Self>, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Start(text) => {
                    fs::remove_file(&self.markers.gone).ok();
                    *locked(&self.proc) = begin_start();
                    let turn = Arc::clone(self);
                    thread::spawn(move || turn.run_turn(&text));
                }
                Action::Emit(event) => {
                    match &event {
                        WorkerEvent::TurnStarted => {
                            self.heartbeat.set_state(ActivityState::Running)
                        }
                        WorkerEvent::TurnEnded(end) => {
                            self.heartbeat.set_state(turn_end_activity(end))
                        }
                        _ => {}
                    }
                    push_event(&self.events, event);
                }
                Action::MarkUnreported => {
                    write_file(&self.markers.unreported, "").ok();
                    write_file(&self.markers.waiting, "").ok();
                }
                Action::PostFailure(problem) => {
                    let at = chrono::Utc::now().to_rfc3339();
                    let agent = HarnessId::Cursor.as_str();
                    self.inbox
                        .post(&prompt_failure_report(agent, &self.id, &at, &problem), 0)
                        .ok();
                }
                Action::Interrupt(_) => self.interrupt_group(),
                Action::InterruptResult { entry, outcome } => {
                    write_interrupt_result(&self.results, &entry, outcome);
                }
                Action::Snapshot => {
                    let turn = self
                        .changes
                        .as_ref()
                        .and_then(WorkerChanges::snapshot_turn_or_warn);
                    *locked(&self.snapshot) = turn;
                }
            }
        }
    }

    fn interrupt_group(self: &Arc<Self>) {
        let plan = {
            let mut state = locked(&self.proc);
            let (next, plan) = on_interrupt(*state);
            *state = next;
            plan
        };
        match plan {
            InterruptPlan::Drop => self.drive(Turns::interrupt_dropped),
            InterruptPlan::Send(group) => self.send_interrupt(group),
            InterruptPlan::Hold => {}
        }
    }

    fn send_interrupt(self: &Arc<Self>, group: Group) {
        self.sigint_turn.store(group.turn, Ordering::SeqCst);
        crate::cursor_owner::signal_group(group.pgid, "INT");
        let shared = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(INTERRUPT_GRACE);
            let target = escalation_target(*locked(&shared.proc), group);
            if let Some(pgid) = target {
                crate::cursor_owner::signal_group(pgid, "TERM");
                thread::sleep(SHUTDOWN_GRACE);
                let state = *locked(&shared.proc);
                if kill_after_term(state, group) {
                    crate::cursor_owner::signal_group(pgid, "KILL");
                    unregister_group(pgid);
                }
            }
        });
    }

    fn run_turn(self: Arc<Self>, text: &str) {
        let (end, gone, usage) = match self.execute(text) {
            Ok((outcome, result, usage)) => (
                turn_end_for(outcome, result),
                outcome == ExitOutcome::ProcessGone,
                usage,
            ),
            Err(message) => (
                TurnEnd::Failed {
                    message,
                    transient: false,
                },
                false,
                None,
            ),
        };
        let closing = self.closing.load(Ordering::SeqCst);
        *locked(&self.snapshot) = None;
        self.drive(|turns| turns.signal(ThreadSignal::TurnEnded(end.clone()), closing));
        if let Some(usage) = &usage {
            let turn_id = locked(&self.snapshot).take();
            self.record_usage(usage, turn_id);
        }
        if gone && !closing {
            let reason = format!("{} -p exited without a result", CURSOR_AGENT.command);
            self.heartbeat.gone(&Scope::Worker, &reason);
            push_event(
                &self.events,
                WorkerEvent::ProcessGone {
                    scope: Scope::Worker,
                    reason,
                },
            );
        }
    }

    fn record_usage(&self, usage: &ModelUsage, turn_id: Option<u64>) {
        let chat = locked(&self.chat).clone();
        record_cursor_usage(
            &CursorTurn {
                worker: &self.id,
                chat: chat.as_deref(),
                store: Some(&self.run.store),
                turn_id,
            },
            usage,
        );
    }

    fn execute(self: &Arc<Self>, text: &str) -> Result<TurnRun, String> {
        if self.closing.load(Ordering::SeqCst) {
            *locked(&self.proc) = on_release();
            return Ok(closed_turn());
        }
        let chat = locked(&self.chat).clone();
        let args = print_args(&PrintArgs {
            workspace: &self.run.worktree,
            model: &self.run.model,
            resume: chat.as_deref(),
        });
        let mut command = Command::new(CURSOR_AGENT.command);
        command
            .args(args)
            .current_dir(&self.run.worktree)
            .envs([inbox_env(&self.run.store)]);
        let (mut child, mut stdin, stdout, stderr) = spawn_piped(command).map_err(|e| {
            *locked(&self.proc) = on_release();
            format!("could not start `{} -p`: {e}", CURSOR_AGENT.command)
        })?;
        let turn = self.spawns.fetch_add(1, Ordering::SeqCst);
        let held = {
            let mut state = locked(&self.proc);
            let group = Group {
                pgid: child.id(),
                turn,
            };
            let (next, held) = on_spawned(*state, group);
            *state = next;
            held
        };
        if self.closing.load(Ordering::SeqCst) {
            self.release_child(&mut child);
            return Ok(closed_turn());
        }
        if let Some(group) = held {
            self.send_interrupt(group);
        }
        drain_stderr(stderr, CappedLog::new(Some(&self.run.store), &self.id));
        if let Err(e) = stdin.write_all(text.as_bytes()) {
            self.release_child(&mut child);
            return Err(format!(
                "could not write the prompt to the cursor agent: {e}"
            ));
        }
        drop(stdin);
        let (lines, incoming) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let mut result = None;
        let mut spend = None;
        let mut result_at: Option<Instant> = None;
        loop {
            let next = match read_plan(result_at.map(|at| at.elapsed()), RESULT_GRACE) {
                ReadPlan::Block => incoming.recv().ok(),
                ReadPlan::Wait(left) => incoming.recv_timeout(left).ok(),
                ReadPlan::Stop => None,
            };
            let Some(line) = next else { break };
            self.heartbeat.touch();
            let event = decode_print_line(&line, &self.run.model);
            self.remember_chat(&event);
            if let PrintEvent::Result { end, usage } = event {
                if result.is_none() {
                    result = Some(end);
                    spend = usage;
                    result_at = Some(Instant::now());
                    let mut state = locked(&self.proc);
                    *state = on_result(*state);
                }
            }
        }
        self.release_child(&mut child);
        let sigint_sent = sigint_hit_turn(self.sigint_turn.load(Ordering::SeqCst), turn);
        Ok((exit_outcome(result.is_some(), sigint_sent), result, spend))
    }

    fn release_child(&self, child: &mut Child) {
        stop_child(child);
        unregister_group(child.id());
        *locked(&self.proc) = on_release();
    }

    fn remember_chat(&self, event: &PrintEvent) {
        let mut chat = locked(&self.chat);
        let next = next_chat(chat.as_deref(), event);
        if next != *chat {
            if let Some(id) = &next {
                let record = SessionRecord {
                    session_id: id.clone(),
                    ..self.run.record.clone()
                };
                write_file(&self.run.session, &record.render()).ok();
            }
            *chat = next;
        }
    }
}

pub(super) struct PrintWorker {
    shared: Arc<Shared>,
    owner: PathBuf,
    state: HeadlessState,
    stopped: bool,
}

impl PrintWorker {
    pub(super) fn start(
        id: &str,
        request: &SpawnRequest,
        files: &HandoffFiles,
        resume: Option<&SessionRecord>,
    ) -> Result<Self, String> {
        if let Some(warning) = agent_version_warning(&CURSOR_AGENT, &request.inbox) {
            report_start_warning(warning, Some(&files.warning));
        }
        let store = request.inbox.parent().ok_or("worker inbox has no store")?;
        let store = fs::canonicalize(store).map_err(|e| e.to_string())?;
        write_mcp_config(id, request, files)?;
        claim_owner(&files.owner)
            .map_err(|e| format!("could not write the {} owner file: {e}", CURSOR_AGENT.name))?;
        let markers = WorkerMarkers::new(&store, id);
        let heartbeat = Heartbeat::new(markers.activity.clone(), markers.gone.clone());
        heartbeat.clear();
        let chat = resume
            .map(|record| record.session_id.clone())
            .filter(|chat| !chat.is_empty());
        let changes = request
            .snapshots
            .then(|| {
                WorkerChanges::open(&WorkerSite {
                    store: store.clone(),
                    id: id.to_string(),
                    worktree: request.worktree.clone(),
                })
            })
            .flatten();
        let shared = Arc::new(Shared {
            id: id.to_string(),
            run: Run {
                worktree: request.worktree.clone(),
                model: print_model(request.model.as_deref()).to_string(),
                session: files.session.clone(),
                record: SessionRecord {
                    session_id: String::new(),
                    cwd: request.worktree.clone(),
                    inbox: request.inbox.clone(),
                    store: Some(store.clone()),
                    model: request.model.clone(),
                    effort: None,
                    permissions: Some(PermissionMode::Yolo),
                },
                store,
            },
            turns: Mutex::new(Turns::default()),
            events: Events::default(),
            heartbeat,
            markers,
            inbox: Inbox::new(request.inbox.clone()),
            results: files.interrupt_result.clone(),
            chat: Mutex::new(chat),
            changes,
            snapshot: Mutex::new(None),
            proc: Mutex::new(TurnProc::Idle),
            turn_seq: AtomicU64::new(0),
            sigint_turn: AtomicU64::new(NO_SIGINT),
            spawns: AtomicU64::new(0),
            closing: AtomicBool::new(false),
        });
        if resume.is_none() {
            write_file(&shared.run.session, &shared.run.record.render()).ok();
            let task = task_prompt(&request.task);
            shared.drive(|turns| turns.accept(task));
        }
        Ok(Self {
            shared,
            owner: files.owner.clone(),
            state: HeadlessState::default(),
            stopped: false,
        })
    }

    pub(super) fn accept(&self, text: String) {
        self.shared.drive(|turns| turns.accept(text));
    }

    pub(super) fn interrupt(&self, text: String, entry: String) {
        self.shared.drive(|turns| turns.interrupt(text, entry));
    }

    pub(super) fn apply_events(&mut self) {
        let events = std::mem::take(&mut *locked(&self.shared.events));
        for event in events {
            let (state, effects) = reduce(&self.state, &event);
            self.state = state;
            for effect in &effects {
                apply_effect(&self.shared.markers, effect);
            }
        }
    }

    pub(super) fn closed(&self) {
        push_event(&self.shared.events, WorkerEvent::Closed);
    }

    pub(super) fn begin_stop(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
    }

    pub(super) fn terminate(&mut self) {
        self.begin_stop();
        if self.stopped {
            return;
        }
        self.stopped = true;
        let group = live_group(*locked(&self.shared.proc));
        if let Some(group) = group {
            stop_groups(&[group], SHUTDOWN_GRACE, || true);
        }
        fs::remove_file(&self.owner).ok();
        if let Some(home) = dirs::home_dir() {
            let real = fs::canonicalize(&self.shared.run.worktree)
                .unwrap_or_else(|_| self.shared.run.worktree.clone());
            sweep_worker_server(&home.join(".cursor/projects"), &real);
        }
    }
}

impl Drop for PrintWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn write_mcp_config(id: &str, request: &SpawnRequest, files: &HandoffFiles) -> Result<(), String> {
    let server = acp_mcp_server(
        &request.exe,
        id,
        &request.inbox,
        &files.owner,
        request.snapshots.then_some(request.worktree.as_path()),
    );
    let path = request.worktree.join(".cursor").join("mcp.json");
    let existing = fs::read_to_string(&path).ok();
    let entry = &claude_mcp_config(&server)["mcpServers"][SERVER_NAME];
    write_file(&path, &cursor_mcp_json(existing.as_deref(), entry))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRACE: Duration = Duration::from_millis(1500);

    #[test]
    fn read_plan_blocks_before_a_result() {
        assert_eq!(read_plan(None, GRACE), ReadPlan::Block);
    }

    #[test]
    fn read_plan_waits_the_rest_of_the_grace_after_a_result() {
        assert_eq!(
            read_plan(Some(Duration::ZERO), GRACE),
            ReadPlan::Wait(GRACE)
        );
        assert_eq!(
            read_plan(Some(Duration::from_millis(500)), GRACE),
            ReadPlan::Wait(Duration::from_millis(1000))
        );
    }

    #[test]
    fn read_plan_stops_when_the_grace_is_spent() {
        assert_eq!(read_plan(Some(GRACE), GRACE), ReadPlan::Stop);
        assert_eq!(
            read_plan(Some(Duration::from_secs(9)), GRACE),
            ReadPlan::Stop
        );
    }

    const PGID: u32 = 42;
    const TURN: u64 = 7;
    const GROUP: Group = Group {
        pgid: PGID,
        turn: TURN,
    };

    #[test]
    fn begin_start_holds_no_interrupt() {
        assert_eq!(begin_start(), TurnProc::Starting { interrupt: false });
    }

    #[test]
    fn interrupt_in_starting_is_held() {
        assert_eq!(
            on_interrupt(TurnProc::Starting { interrupt: false }),
            (TurnProc::Starting { interrupt: true }, InterruptPlan::Hold)
        );
        assert_eq!(
            on_interrupt(TurnProc::Starting { interrupt: true }),
            (TurnProc::Starting { interrupt: true }, InterruptPlan::Hold)
        );
    }

    #[test]
    fn held_interrupt_is_delivered_when_the_child_exists() {
        assert_eq!(
            on_spawned(TurnProc::Starting { interrupt: true }, GROUP),
            (
                TurnProc::Running {
                    pgid: PGID,
                    turn: TURN
                },
                Some(GROUP)
            )
        );
    }

    #[test]
    fn spawn_without_a_held_interrupt_sends_nothing() {
        assert_eq!(
            on_spawned(TurnProc::Starting { interrupt: false }, GROUP),
            (
                TurnProc::Running {
                    pgid: PGID,
                    turn: TURN
                },
                None
            )
        );
    }

    #[test]
    fn interrupt_in_running_sends_to_the_group() {
        assert_eq!(
            on_interrupt(TurnProc::Running {
                pgid: PGID,
                turn: TURN
            }),
            (
                TurnProc::Running {
                    pgid: PGID,
                    turn: TURN
                },
                InterruptPlan::Send(GROUP)
            )
        );
    }

    #[test]
    fn interrupt_in_grace_is_dropped() {
        assert_eq!(
            on_interrupt(TurnProc::Grace {
                pgid: PGID,
                turn: TURN
            }),
            (
                TurnProc::Grace {
                    pgid: PGID,
                    turn: TURN
                },
                InterruptPlan::Drop
            )
        );
    }

    #[test]
    fn a_signal_counts_only_for_the_turn_it_hit() {
        assert!(sigint_hit_turn(4, 4));
        assert!(!sigint_hit_turn(3, 4));
        assert!(!sigint_hit_turn(NO_SIGINT, 0));
        assert!(!sigint_hit_turn(NO_SIGINT, u64::MAX - 1));
    }

    #[test]
    fn interrupt_in_idle_is_dropped() {
        assert_eq!(
            on_interrupt(TurnProc::Idle),
            (TurnProc::Idle, InterruptPlan::Drop)
        );
    }

    #[test]
    fn release_returns_to_idle_and_drops_the_held_interrupt() {
        assert_eq!(on_release(), TurnProc::Idle);
        assert_eq!(
            on_interrupt(on_release()),
            (TurnProc::Idle, InterruptPlan::Drop)
        );
    }

    #[test]
    fn first_result_moves_running_to_grace() {
        assert_eq!(
            on_result(TurnProc::Running {
                pgid: PGID,
                turn: TURN
            }),
            TurnProc::Grace {
                pgid: PGID,
                turn: TURN
            }
        );
    }

    #[test]
    fn result_changes_no_other_state() {
        assert_eq!(on_result(TurnProc::Idle), TurnProc::Idle);
        assert_eq!(
            on_result(TurnProc::Starting { interrupt: true }),
            TurnProc::Starting { interrupt: true }
        );
        assert_eq!(
            on_result(TurnProc::Grace {
                pgid: PGID,
                turn: TURN
            }),
            TurnProc::Grace {
                pgid: PGID,
                turn: TURN
            }
        );
    }

    #[test]
    fn live_group_exists_only_with_a_child() {
        assert_eq!(live_group(TurnProc::Idle), None);
        assert_eq!(live_group(TurnProc::Starting { interrupt: true }), None);
        assert_eq!(
            live_group(TurnProc::Running {
                pgid: PGID,
                turn: TURN
            }),
            Some(PGID)
        );
        assert_eq!(
            live_group(TurnProc::Grace {
                pgid: PGID,
                turn: TURN
            }),
            Some(PGID)
        );
    }

    #[test]
    fn escalation_targets_only_the_same_running_turn() {
        assert_eq!(
            escalation_target(
                TurnProc::Running {
                    pgid: PGID,
                    turn: TURN
                },
                GROUP
            ),
            Some(PGID)
        );
        assert_eq!(
            escalation_target(
                TurnProc::Grace {
                    pgid: PGID,
                    turn: TURN
                },
                GROUP
            ),
            None
        );
        assert_eq!(escalation_target(TurnProc::Idle, GROUP), None);
        assert_eq!(
            escalation_target(TurnProc::Starting { interrupt: true }, GROUP),
            None
        );
        assert_eq!(
            escalation_target(
                TurnProc::Running {
                    pgid: PGID + 1,
                    turn: TURN
                },
                GROUP
            ),
            None
        );
    }

    #[test]
    fn a_reused_pid_in_a_newer_turn_is_not_escalated() {
        assert_eq!(
            escalation_target(
                TurnProc::Running {
                    pgid: PGID,
                    turn: TURN + 1
                },
                GROUP
            ),
            None
        );
    }

    #[test]
    fn kill_follows_term_only_while_the_same_turn_runs() {
        let running = TurnProc::Running {
            pgid: PGID,
            turn: TURN,
        };
        assert!(kill_after_term(running, GROUP));
        assert!(!kill_after_term(
            TurnProc::Grace {
                pgid: PGID,
                turn: TURN
            },
            GROUP
        ));
        assert!(!kill_after_term(TurnProc::Idle, GROUP));
        assert!(!kill_after_term(
            TurnProc::Running {
                pgid: PGID,
                turn: TURN + 1
            },
            GROUP
        ));
    }
}
