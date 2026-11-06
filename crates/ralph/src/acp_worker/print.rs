use super::{agent_version_warning, SHUTDOWN_GRACE};
use crate::acp_log::{drain_stderr, CappedLog};
use crate::cursor_owner::{claim_owner, report_start_warning, write_file, write_interrupt_result};
use crate::cursor_sweep::sweep_worker_server;
use crate::cursor_usage::{record_cursor_usage, CursorTurn};
use crate::subprocess::unregister_group;
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::watches::locked;
use crate::worker_events::{apply_effect, push_event, Events};
use crate::worker_mcp::Inbox;
use crate::workers::headless::Heartbeat;
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
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const INTERRUPT_GRACE: Duration = Duration::from_secs(3);

type TurnRun = (ExitOutcome, Option<TurnEnd>, Option<ModelUsage>);

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
    group: AtomicU32,
    turn_seq: AtomicU64,
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
                Action::Snapshot => {}
            }
        }
    }

    fn interrupt_group(self: &Arc<Self>) {
        let group = self.group.load(Ordering::SeqCst);
        if group == 0 {
            return;
        }
        crate::cursor_owner::signal_group(group, "INT");
        let shared = Arc::clone(self);
        thread::spawn(move || {
            thread::sleep(INTERRUPT_GRACE);
            if shared.group.load(Ordering::SeqCst) == group {
                stop_groups(&[group], SHUTDOWN_GRACE, || true);
            }
        });
    }

    fn run_turn(self: Arc<Self>, text: &str) {
        let (end, gone) = match self.execute(text) {
            Ok((outcome, result, usage)) => {
                if let Some(usage) = &usage {
                    self.record_usage(usage);
                }
                (
                    turn_end_for(outcome, result),
                    outcome == ExitOutcome::ProcessGone,
                )
            }
            Err(message) => (
                TurnEnd::Failed {
                    message,
                    transient: false,
                },
                false,
            ),
        };
        let closing = self.closing.load(Ordering::SeqCst);
        self.drive(|turns| turns.signal(ThreadSignal::TurnEnded(end.clone()), closing));
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

    fn record_usage(&self, usage: &ModelUsage) {
        let chat = locked(&self.chat).clone();
        record_cursor_usage(
            &CursorTurn {
                worker: &self.id,
                chat: chat.as_deref(),
                store: Some(&self.run.store),
                turn_id: None,
            },
            usage,
        );
    }

    fn execute(&self, text: &str) -> Result<TurnRun, String> {
        let chat = locked(&self.chat).clone();
        let args = print_args(&PrintArgs {
            workspace: &self.run.worktree,
            model: &self.run.model,
            resume: chat.as_deref(),
            mode: PermissionMode::Yolo,
        });
        let mut command = Command::new(CURSOR_AGENT.command);
        command
            .args(args)
            .current_dir(&self.run.worktree)
            .envs([inbox_env(&self.run.store)]);
        let (mut child, mut stdin, stdout, stderr) = spawn_piped(command)
            .map_err(|e| format!("could not start `{} -p`: {e}", CURSOR_AGENT.command))?;
        self.group.store(child.id(), Ordering::SeqCst);
        drain_stderr(stderr, CappedLog::new(Some(&self.run.store), &self.id));
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("could not write the prompt to the cursor agent: {e}"))?;
        drop(stdin);
        let mut result = None;
        let mut spend = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            self.heartbeat.touch();
            let event = decode_print_line(&line, &self.run.model);
            self.remember_chat(&event);
            if let PrintEvent::Result { end, usage } = event {
                result = Some(end);
                spend = usage;
            }
        }
        stop_child(&mut child);
        unregister_group(child.id());
        self.group.store(0, Ordering::SeqCst);
        let sigint_sent = locked(&self.turns).interrupt_sent();
        Ok((exit_outcome(result.is_some(), sigint_sent), result, spend))
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
            group: AtomicU32::new(0),
            turn_seq: AtomicU64::new(0),
            closing: AtomicBool::new(false),
        });
        if resume.is_none() {
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
        let group = self.shared.group.load(Ordering::SeqCst);
        if group != 0 {
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
