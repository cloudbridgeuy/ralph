use crate::acp_log::{drain_stderr, CappedLog, LoggedWrite};
use crate::acp_worker::{apply_effect, push_event, Events};
use crate::codex_usage::CodexUsage;
use crate::config::{configured_codex, configured_permission_mode, AppConfig};
use crate::cursor_owner::{
    claim_owner, now_secs, queued_entries, report_start_warning, request_ids, sweep_owner,
    take_answer, write_file, write_interrupt_result, write_owner_record,
};
use crate::permission_queue::{host_process_token, PermissionQueue};
use crate::process_tree::{snapshot_tree, sweep_tree};
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::verified_agents;
use crate::warn::warn;
use crate::watches::locked;
use crate::worker_changes::WorkerChanges;
use crate::worker_mcp::Inbox;
use crate::workers::headless::{HeadlessHost, Heartbeat};
use ralph_core::snapshot::{snapshots_enabled, WorkerSite};
use ralph_core::worker::{
    acp_mcp_server, activity_path, answer_server_request, check_effort, check_login, check_model,
    check_version, classify, codex_close_plan, codex_resume_decision, decide, decode_thread,
    decode_turn_id, decode_version, describe, gone_path, initialize_params, is_unmapped, model_ids,
    model_list_params, next_cursor, prompt_failure_report, ready_path, record_selection, reduce,
    reject_server_request, request_stale, resolve_selection, resolved_request_id, response_body,
    session_saved, task_prompt, thread_resume_params, thread_start_params, turn_interrupt_params,
    turn_start_params, turn_stop, yolo_option, Action, ApprovalKind, BlockedContent, Capabilities,
    CodexCloseStep, HandoffFiles, HarnessId, HeadlessState, HeadlessWorker, Inbound, Incoming,
    McpStartup, McpVerdict, PermissionAnswer, PermissionMode, Resume, ResumeFacts, Rpc, Scope,
    Selection, SessionRecord, SpawnRequest, ThreadInfo, ThreadSignal, TurnEnd, Turns,
    WorkerCommand, WorkerEvent, WorkerMarkers, ACCOUNT_READ, CODEX_ANSWER_DIR, CODEX_CLOSE_DIR,
    CODEX_CLOSE_TURN_WAIT, CODEX_COMMAND, CODEX_INTERRUPT_DIR, CODEX_OWNER_DIR, CODEX_PROMPT_DIR,
    CODEX_SESSION_DIR, CODEX_SPAWN_DIR, INITIALIZE, INITIALIZED, MODEL_LIST, MODEL_LIST_MAX_PAGES,
    SERVER_NAME, THREAD_RESUME, THREAD_START, TURN_INTERRUPT, TURN_START,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) mod pipes;
mod resume;
mod verify;

use pipes::{logged_stdin, pipe_lines};
pub(crate) use verify::verify;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
const REPLY_POLL: Duration = Duration::from_millis(100);
const CLOSE_POLL: Duration = Duration::from_millis(20);
const EXIT_REASON: &str = "codex app-server exited";

pub(crate) type Stdin = Arc<Mutex<Option<LoggedWrite<ChildStdin>>>>;

struct CodexReply {
    id: Value,
    kind: ApprovalKind,
}

type Permissions = PermissionQueue<CodexReply>;

enum Stop {
    Timeout,
    Gone,
    Bad(String),
}

impl Stop {
    fn message(self) -> String {
        self.message_within(None)
    }

    fn message_within(self, verify_limit: Option<Duration>) -> String {
        match self {
            Self::Timeout => match verify_limit {
                Some(limit) => format!("codex did not answer within the {limit:?} verify limit"),
                None => format!("codex was not ready after {READY_TIMEOUT:?}"),
            },
            Self::Gone => "codex exited before it was ready".to_string(),
            Self::Bad(message) => message,
        }
    }
}

struct Wire {
    rpc: Rpc,
    stdin: Stdin,
    lines: mpsc::Receiver<String>,
    startup: McpStartup,
    heartbeat: Arc<Heartbeat>,
    closing: Arc<AtomicBool>,
    verify_limit: Option<Duration>,
}

fn exit_reason(status: ExitStatus) -> String {
    format!("{CODEX_COMMAND} app-server exited ({status})")
}

pub(crate) fn report_exit(
    heartbeat: &Heartbeat,
    events: &Events,
    exited: &AtomicBool,
    closing: &AtomicBool,
    reason: &str,
) {
    if exited.swap(true, Ordering::SeqCst) || closing.load(Ordering::SeqCst) {
        return;
    }
    heartbeat.gone(&Scope::Worker, reason);
    push_event(
        events,
        WorkerEvent::ProcessGone {
            scope: Scope::Worker,
            reason: reason.to_string(),
        },
    );
}

fn write_line(stdin: &Stdin, line: &str) -> Result<(), Stop> {
    let mut guard = locked(stdin);
    let stdin = guard.as_mut().ok_or(Stop::Gone)?;
    writeln!(stdin, "{line}")
        .and_then(|()| stdin.flush())
        .map_err(|e| Stop::Bad(format!("could not write to codex: {e}")))
}

impl Wire {
    fn send_line(&self, line: &str) -> Result<(), Stop> {
        write_line(&self.stdin, line)
    }

    fn next(&mut self, deadline: Option<Instant>) -> Result<Incoming, Stop> {
        match self.rpc.next_deferred() {
            Some(incoming) => Ok(incoming),
            None => self.next_frame(deadline),
        }
    }

    fn next_frame(&mut self, deadline: Option<Instant>) -> Result<Incoming, Stop> {
        let line = match deadline {
            Some(deadline) => self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| match e {
                    RecvTimeoutError::Timeout => Stop::Timeout,
                    RecvTimeoutError::Disconnected => Stop::Gone,
                })?,
            None => self.lines.recv().map_err(|_| Stop::Gone)?,
        };
        self.heartbeat.touch();
        let incoming = self
            .rpc
            .receive(&line)
            .map_err(|e| Stop::Bad(e.to_string()))?;
        if let Incoming::Notification { method, params } = &incoming {
            self.startup.observe(method, params);
        }
        Ok(incoming)
    }

    fn answer(&self, incoming: &Incoming) -> Result<(), Stop> {
        match incoming {
            Incoming::ServerRequest { id, method, .. } => {
                self.send_line(&reject_server_request(id, method))
            }
            _ => Ok(()),
        }
    }

    fn call(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, String> {
        let (id, line) = self.rpc.request(method, params);
        let limit = self.verify_limit;
        self.send_line(&line)
            .map_err(|stop| stop.message_within(limit))?;
        loop {
            if self.closing.load(Ordering::SeqCst) {
                return Err(format!("codex {method} failed: worker closed"));
            }
            let slice = deadline.min(Instant::now() + REPLY_POLL);
            let incoming = match self.next_frame(Some(slice)) {
                Err(Stop::Timeout) if Instant::now() < deadline => continue,
                other => other.map_err(|stop| stop.message_within(limit))?,
            };
            if let Some(outcome) = self.rpc.settle(id, incoming) {
                return outcome.map_err(|e| format!("codex {method} failed: {}", e.message));
            }
        }
    }
}

fn list_models(wire: &mut Wire, deadline: Instant) -> Result<Value, String> {
    let mut data = Vec::new();
    let mut cursor = None;
    for _ in 0..MODEL_LIST_MAX_PAGES {
        let page = wire.call(MODEL_LIST, model_list_params(cursor.as_deref()), deadline)?;
        data.extend(page["data"].as_array().cloned().unwrap_or_default());
        cursor = next_cursor(&page);
        if cursor.is_none() {
            break;
        }
    }
    Ok(json!({"data": data}))
}

fn handshake(
    wire: &mut Wire,
    selection: &Selection,
    open: (&str, Value),
    deadline: Instant,
    warning_file: Option<&Path>,
) -> Result<ThreadInfo, String> {
    let init = wire.call(
        INITIALIZE,
        initialize_params(env!("CARGO_PKG_VERSION")),
        deadline,
    )?;
    wire.send_line(&Rpc::notification(INITIALIZED))
        .map_err(Stop::message)?;
    let version = decode_version(&init)
        .ok_or("could not read the codex version from its initialize reply")?;
    if let Some(warning) = check_version(&version, &verified_agents::load())? {
        report_start_warning(warning, warning_file);
    }
    check_login(&wire.call(ACCOUNT_READ, json!({}), deadline)?)?;
    let listed = list_models(wire, deadline)?;
    check_model(selection.model.as_deref(), &model_ids(&listed))?;
    check_effort(selection, &listed)?;
    let (method, params) = open;
    let started = wire.call(method, params, deadline)?;
    let thread = decode_thread(&started).ok_or("codex did not return a thread id")?;
    loop {
        match wire.startup.verdict(&[SERVER_NAME]) {
            McpVerdict::Ready => return Ok(thread),
            McpVerdict::Failed { server, detail } => {
                return Err(format!(
                    "codex could not start the {server} MCP server: {detail}"
                ))
            }
            McpVerdict::Waiting => {
                let incoming = wire.next(Some(deadline)).map_err(Stop::message)?;
                wire.answer(&incoming).map_err(Stop::message)?;
            }
        }
    }
}

struct Pump {
    wire: Wire,
    thread: String,
    turns: Turns,
    events: Events,
    markers: WorkerMarkers,
    closing: Arc<AtomicBool>,
    replies: mpsc::Receiver<Inbound>,
    results: PathBuf,
    interrupt_call: Option<u64>,
    permissions: Arc<Permissions>,
    mode: PermissionMode,
    selection: Selection,
    pending_record: Option<(PathBuf, SessionRecord)>,
    inbox: Inbox,
    changes: Option<WorkerChanges>,
    id: String,
    wound_down: bool,
    log: Arc<CappedLog>,
    unmapped: HashSet<String>,
    usage: Arc<CodexUsage>,
    snapshot_turn: Option<u64>,
}

impl Pump {
    fn wind_down(&mut self) {
        self.wound_down = true;
        self.wire.rpc.fail_pending();
        let actions = self.turns.close();
        self.settle(actions);
    }

    fn save_record(&mut self) {
        if self.closing.load(Ordering::SeqCst) {
            return;
        }
        if let Some((path, record)) = self.pending_record.take() {
            if let Err(e) = write_file(&path, &record.render()) {
                warn(format!("could not write the codex session file: {e}"));
            }
        }
    }

    fn perform(&mut self, actions: Vec<Action>) -> Result<(), String> {
        for action in actions {
            match action {
                Action::Start(text) => {
                    let params = turn_start_params(&self.thread, &text, &self.selection);
                    let started =
                        self.wire
                            .call(TURN_START, params, Instant::now() + READY_TIMEOUT)?;
                    let bound = self.turns.bind_turn(decode_turn_id(&started));
                    self.perform(bound)?;
                }
                Action::Interrupt(turn) => {
                    let params = turn_interrupt_params(&self.thread, &turn);
                    let (id, line) = self.wire.rpc.request(TURN_INTERRUPT, params);
                    self.interrupt_call = Some(id);
                    self.wire.send_line(&line).map_err(Stop::message)?;
                    self.permissions.drain();
                }
                Action::InterruptResult { entry, cancelled } => {
                    write_interrupt_result(&self.results, &entry, cancelled);
                }
                Action::Emit(event) => {
                    match &event {
                        WorkerEvent::TurnStarted => {
                            self.snapshot_turn = None;
                            self.usage.start_turn();
                            self.wire.heartbeat.set_running(true);
                        }
                        WorkerEvent::TurnEnded(end) => {
                            self.wire.heartbeat.set_running(false);
                            self.usage.flush(self.snapshot_turn.take());
                            if session_saved(turn_stop(end), true) {
                                self.save_record();
                            }
                        }
                        _ => {}
                    }
                    push_event(&self.events, event);
                }
                Action::MarkUnreported => {
                    write_file(&self.markers.unreported, "").ok();
                    write_file(&self.markers.waiting, "").ok();
                }
                Action::Snapshot => {
                    if let Some(changes) = &self.changes {
                        self.snapshot_turn = changes.snapshot_turn_or_warn();
                    }
                }
                Action::PostFailure(problem) => {
                    let at = chrono::Utc::now().to_rfc3339();
                    let report =
                        prompt_failure_report(HarnessId::Codex.as_str(), &self.id, &at, &problem);
                    self.inbox.post(&report, 0).ok();
                }
            }
        }
        Ok(())
    }

    fn settle(&mut self, mut actions: Vec<Action>) {
        while let Err(message) = self.perform(actions) {
            let failed = ThreadSignal::TurnEnded(TurnEnd::Failed {
                message,
                transient: false,
            });
            actions = self
                .turns
                .signal(failed, self.closing.load(Ordering::SeqCst));
        }
    }

    fn server_request(&mut self, id: Value, method: &str, params: &Value) -> Result<(), Stop> {
        let stale = self.turns.is_stale_request(method, params);
        let Some(approval) = classify(method, params).filter(|_| !stale) else {
            return self.wire.send_line(&reject_server_request(&id, method));
        };
        let auto = Some(self.mode)
            .filter(|mode| *mode == PermissionMode::Yolo)
            .and_then(|_| yolo_option(&approval));
        match auto {
            Some(option) => {
                let body = response_body(approval.kind, &option.option_id);
                self.wire.send_line(&answer_server_request(&id, &body))
            }
            None => {
                let content = BlockedContent {
                    request_id: id.as_str().map_or_else(|| id.to_string(), str::to_string),
                    ..approval.content
                };
                let reply = CodexReply {
                    id,
                    kind: approval.kind,
                };
                self.permissions.enqueue(reply, content);
                Ok(())
            }
        }
    }

    fn run(mut self) {
        loop {
            if !self.wound_down && self.closing.load(Ordering::SeqCst) {
                self.wind_down();
            }
            while let Ok(inbound) = self.replies.try_recv() {
                let actions = match inbound {
                    Inbound::Prompt(text) => self.turns.accept(text),
                    Inbound::Interrupt(text, entry) => self.turns.interrupt(text, entry),
                };
                self.settle(actions);
            }
            match self.wire.next(Some(Instant::now() + REPLY_POLL)) {
                Err(Stop::Gone) => return,
                Ok(Incoming::Notification { method, params }) => {
                    if let Some(id) = resolved_request_id(&method, &params) {
                        self.permissions.settle(|reply| reply.id == id);
                    }
                    if is_unmapped(&method) && self.unmapped.insert(method.clone()) {
                        self.log.append("unmapped", method.as_bytes());
                    }
                    let in_turn = self.turns.owns_event(&method, &params);
                    self.usage.observe(&method, &params, in_turn);
                    let closing = self.closing.load(Ordering::SeqCst);
                    let actions = self.turns.notification(&method, &params, closing);
                    self.settle(actions);
                }
                Ok(Incoming::Response {
                    id,
                    outcome: Err(error),
                }) if self.interrupt_call == Some(id) => {
                    let actions = self.turns.interrupt_failed(error.code);
                    self.settle(actions);
                }
                Ok(Incoming::ServerRequest { id, method, params }) => {
                    if let Err(Stop::Gone) = self.server_request(id, &method, &params) {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

pub enum Launch {
    Fresh(SessionRecord),
    Resume(SessionRecord),
}

pub struct Slot<'a> {
    pub id: &'a str,
    pub owner: &'a Path,
    pub session: &'a Path,
    pub results: &'a Path,
    pub launch: Launch,
    pub markers: WorkerMarkers,
    pub starting: &'a Mutex<HashMap<String, u32>>,
    pub warning: Option<&'a Path>,
}

pub struct CodexWorker {
    child: Child,
    stdin: Stdin,
    finished: mpsc::Receiver<()>,
    owner: PathBuf,
    closing: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
    heartbeat: Arc<Heartbeat>,
    done: bool,
    caps: Capabilities,
    markers: WorkerMarkers,
    state: HeadlessState,
    events: Events,
    replies: mpsc::Sender<Inbound>,
    permissions: Arc<Permissions>,
    log: Arc<CappedLog>,
    usage: Arc<CodexUsage>,
}

impl CodexWorker {
    pub fn start(request: &SpawnRequest, slot: Slot) -> Result<Self, String> {
        let Slot {
            id,
            owner,
            session,
            results,
            launch,
            markers,
            starting,
            warning,
        } = slot;
        let mut command = Command::new(CODEX_COMMAND);
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(&request.worktree);
        let (child, stdin, stdout, stderr) = spawn_piped(command)
            .map_err(|e| format!("could not start `{CODEX_COMMAND} app-server`: {e}"))?;
        let log = CappedLog::new(request.inbox.parent(), id);
        drain_stderr(stderr, Arc::clone(&log));
        let stdin = logged_stdin(stdin, &log);
        let (lines, finished) = pipe_lines(stdout, &log);
        let heartbeat = Heartbeat::new(markers.activity.clone(), markers.gone.clone());
        heartbeat.clear();
        let exited = Arc::new(AtomicBool::new(false));
        let closing = Arc::new(AtomicBool::new(false));
        let (replies, reply_rx) = mpsc::channel();
        let events = Events::default();
        let permissions = Arc::new(PermissionQueue::new(
            host_process_token(),
            id.to_string(),
            Inbox::new(request.inbox.clone()),
            Arc::clone(&events),
        ));
        let worker = Self {
            child,
            stdin: Arc::clone(&stdin),
            finished,
            owner: owner.to_path_buf(),
            closing: Arc::clone(&closing),
            exited: Arc::clone(&exited),
            heartbeat: Arc::clone(&heartbeat),
            done: false,
            caps: ralph_core::worker::capabilities(HarnessId::Codex),
            markers: markers.clone(),
            state: HeadlessState::default(),
            events: Arc::clone(&events),
            replies,
            permissions: Arc::clone(&permissions),
            log,
            usage: Arc::new(CodexUsage::new(id, request.inbox.parent())),
        };
        write_owner_record(owner, worker.child.id(), "codex")?;
        locked(starting).insert(id.to_string(), worker.child.id());
        let mut wire = Wire {
            rpc: Rpc::default(),
            stdin,
            lines,
            startup: McpStartup::default(),
            heartbeat: Arc::clone(&heartbeat),
            closing: Arc::clone(&closing),
            verify_limit: None,
        };
        let server = acp_mcp_server(
            &request.exe,
            id,
            &request.inbox,
            owner,
            request.snapshots.then_some(request.worktree.as_path()),
        );
        let deadline = Instant::now() + READY_TIMEOUT;
        let configured_mode =
            || configured_permission_mode("codex", |config| config.codex.permissions_setting());
        let (selection, open, mode) = match &launch {
            Launch::Fresh(_) => {
                let codex = configured_codex();
                let selection = resolve_selection(
                    request.model.as_deref(),
                    codex.model.as_deref(),
                    codex.effort.as_deref(),
                );
                let open = (
                    THREAD_START,
                    thread_start_params(&request.worktree, &selection, &server),
                );
                (selection, open, configured_mode())
            }
            Launch::Resume(record) => (
                record_selection(record),
                (THREAD_RESUME, thread_resume_params(record, &server)),
                record.permissions.unwrap_or_else(configured_mode),
            ),
        };
        let thread = handshake(&mut wire, &selection, open, deadline, warning)?;
        worker
            .usage
            .bind(&thread.id, thread.model.clone().or(selection.model.clone()));
        let pending_record = match &launch {
            Launch::Fresh(template) => Some((
                session.to_path_buf(),
                SessionRecord {
                    session_id: thread.id.clone(),
                    model: thread.model.or(selection.model.clone()),
                    effort: thread.effort.or(selection.effort.clone()),
                    permissions: Some(mode),
                    ..template.clone()
                },
            )),
            Launch::Resume(record) if record.session_id != thread.id => {
                return Err(format!(
                    "codex resumed thread {} instead of {}",
                    thread.id, record.session_id
                ))
            }
            Launch::Resume(_) => None,
        };
        let mut pump = Pump {
            wire,
            thread: thread.id,
            turns: Turns::default(),
            events: Arc::clone(&worker.events),
            markers: markers.clone(),
            closing,
            replies: reply_rx,
            results: results.to_path_buf(),
            interrupt_call: None,
            permissions,
            mode,
            selection,
            pending_record,
            inbox: Inbox::new(request.inbox.clone()),
            changes: request
                .inbox
                .parent()
                .filter(|_| request.snapshots)
                .and_then(|store| {
                    WorkerChanges::open(&WorkerSite {
                        store: store.to_path_buf(),
                        id: id.to_string(),
                        worktree: request.worktree.clone(),
                    })
                }),
            id: id.to_string(),
            wound_down: false,
            log: Arc::clone(&worker.log),
            unmapped: HashSet::new(),
            usage: Arc::clone(&worker.usage),
            snapshot_turn: None,
        };
        if matches!(launch, Launch::Fresh(_)) {
            let first = pump.turns.accept(task_prompt(&request.task));
            pump.perform(first)?;
        }
        let closing = Arc::clone(&pump.closing);
        let events = Arc::clone(&pump.events);
        thread::spawn(move || {
            pump.run();
            report_exit(&heartbeat, &events, &exited, &closing, EXIT_REASON);
        });
        Ok(worker)
    }

    fn reap_exit(&mut self) {
        if self.done {
            return;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.usage.flush(None);
            report_exit(
                &self.heartbeat,
                &self.events,
                &self.exited,
                &self.closing,
                &exit_reason(status),
            );
        }
    }

    fn apply_events(&mut self) {
        for event in self.poll() {
            let (state, effects) = reduce(&self.state, &event);
            self.state = state;
            for effect in &effects {
                apply_effect(&self.markers, effect);
            }
        }
    }

    fn answer_permission(&self, answer: &PermissionAnswer) -> Result<(), String> {
        let (reply, option) = self
            .permissions
            .take_answer(&answer.request_id, &answer.option_id)?;
        let body = response_body(reply.kind, &option.option_id);
        write_line(&self.stdin, &answer_server_request(&reply.id, &body)).map_err(Stop::message)
    }

    fn interrupt(&self, text: String, entry: &str) {
        self.replies
            .send(Inbound::Interrupt(text, entry.to_string()))
            .ok();
    }

    fn terminate(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        let mut tree = Vec::new();
        for step in codex_close_plan(self.heartbeat.running()) {
            match step {
                CodexCloseStep::WindDown => {
                    self.closing.store(true, Ordering::SeqCst);
                    self.permissions.drain();
                }
                CodexCloseStep::AwaitTurn => {
                    let deadline = Instant::now() + CODEX_CLOSE_TURN_WAIT;
                    while self.heartbeat.running() && Instant::now() < deadline {
                        thread::sleep(CLOSE_POLL);
                    }
                }
                CodexCloseStep::SnapshotTree => tree = snapshot_tree(self.child.id()),
                CodexCloseStep::DropStdin => {
                    locked(&self.stdin).take();
                    self.finished.recv_timeout(SHUTDOWN_GRACE).ok();
                }
                CodexCloseStep::StopGroup => stop_child(&mut self.child),
                CodexCloseStep::SweepTree => {
                    let survivors = sweep_tree(&tree);
                    if !survivors.is_empty() {
                        self.log.append(
                            "sweep",
                            format!("survivors after KILL: {survivors:?}").as_bytes(),
                        );
                    }
                }
                CodexCloseStep::RemoveOwner => {
                    fs::remove_file(&self.owner).ok();
                }
            }
        }
        self.usage.flush(None);
    }
}

impl HeadlessWorker for CodexWorker {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn send(&mut self, command: WorkerCommand) -> Result<(), String> {
        let command = decide(&self.caps, command).map_err(|event| match event {
            WorkerEvent::Unsupported { reason, .. } => reason,
            other => format!("{other:?}"),
        })?;
        match command {
            WorkerCommand::Close => {
                self.terminate();
                push_event(&self.events, WorkerEvent::Closed);
                Ok(())
            }
            WorkerCommand::Prompt(text) => self
                .replies
                .send(Inbound::Prompt(text))
                .map_err(|_| "codex worker is not running".to_string()),
            WorkerCommand::AnswerPermission(answer) => self.answer_permission(&answer),
            WorkerCommand::Spawn(_) | WorkerCommand::Resume | WorkerCommand::Interrupt(_) => {
                Err(format!(
                    "not sent: the host handles it ({})",
                    describe(&self.caps, command.feature())
                ))
            }
        }
    }

    fn poll(&mut self) -> Vec<WorkerEvent> {
        self.reap_exit();
        std::mem::take(&mut *locked(&self.events))
    }
}

impl Drop for CodexWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub struct CodexHost {
    store_dir: PathBuf,
    workers: Mutex<HashMap<String, CodexWorker>>,
    starting: Mutex<HashMap<String, u32>>,
    closed: AtomicBool,
}

impl CodexHost {
    pub fn new(store_dir: PathBuf) -> Arc<Self> {
        for id in request_ids(&store_dir, CODEX_OWNER_DIR) {
            sweep_owner(&HandoffFiles::codex(&store_dir, &id));
        }
        Arc::new(Self {
            store_dir,
            workers: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    fn files(&self, id: &str) -> HandoffFiles {
        HandoffFiles::codex(&self.store_dir, id)
    }

    fn poll(&self) {
        for worker in locked(&self.workers).values_mut() {
            worker.apply_events();
        }
        for id in request_ids(&self.store_dir, CODEX_CLOSE_DIR) {
            self.close(&id);
        }
        for id in request_ids(&self.store_dir, CODEX_ANSWER_DIR) {
            self.answer(&id);
        }
        for id in request_ids(&self.store_dir, CODEX_PROMPT_DIR) {
            self.deliver_queued(
                &id,
                |files| &files.prompt,
                |worker, text, _| {
                    if let Err(message) = worker.send(WorkerCommand::Prompt(text)) {
                        warn(message);
                    }
                },
            );
        }
        for id in request_ids(&self.store_dir, CODEX_INTERRUPT_DIR) {
            self.deliver_queued(
                &id,
                |files| &files.interrupt,
                |worker, text, entry| worker.interrupt(text, entry),
            );
        }
        for id in request_ids(&self.store_dir, CODEX_SPAWN_DIR) {
            self.spawn(&id);
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut drained: Vec<(String, CodexWorker)> = locked(&self.workers).drain().collect();
        thread::scope(|scope| {
            for (_, worker) in &mut drained {
                scope.spawn(|| worker.terminate());
            }
        });
        for (id, _) in &drained {
            self.forget(id);
        }
        let starting: Vec<(String, u32)> = locked(&self.starting).drain().collect();
        if starting.is_empty() {
            return;
        }
        let groups: Vec<u32> = starting.iter().map(|(_, group)| *group).collect();
        stop_groups(&groups, SHUTDOWN_GRACE, || true);
        for (id, _) in &starting {
            self.forget(id);
        }
    }

    fn answer(&self, id: &str) {
        take_answer(&self.files(id), |text| {
            PermissionAnswer::decode(text).and_then(|answer| {
                locked(&self.workers)
                    .get_mut(id)
                    .ok_or_else(|| "worker has no running codex agent".to_string())
                    .and_then(|worker| worker.send(WorkerCommand::AnswerPermission(answer)))
            })
        });
    }

    fn deliver_queued(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&mut CodexWorker, String, &str),
    ) {
        let starting = locked(&self.starting).contains_key(id);
        if starting && !locked(&self.workers).contains_key(id) {
            return;
        }
        for entry in queued_entries(dir(&self.files(id))) {
            let name = entry
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let (Ok(text), Some(worker)) = (
                fs::read_to_string(&entry),
                locked(&self.workers).get_mut(id),
            ) {
                deliver(worker, text, &name);
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn forget(&self, id: &str) {
        let files = self.files(id);
        fs::remove_file(&files.owner).ok();
        fs::remove_file(&files.answer).ok();
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_dir_all(&files.interrupt).ok();
        fs::remove_dir_all(&files.interrupt_result).ok();
        fs::remove_file(activity_path(&self.store_dir, id)).ok();
        fs::remove_file(gone_path(&self.store_dir, id)).ok();
    }

    fn close(&self, id: &str) {
        let removed = locked(&self.workers).remove(id);
        if let Some(mut worker) = removed {
            worker.send(WorkerCommand::Close).ok();
            worker.apply_events();
        }
        let files = self.files(id);
        self.forget(id);
        fs::remove_file(ready_path(&self.store_dir, id)).ok();
        fs::remove_file(&files.session).ok();
        fs::remove_file(&files.close).ok();
    }

    fn spawn(&self, id: &str) {
        let files = self.files(id);
        let request = fs::read_to_string(&files.spawn)
            .ok()
            .and_then(|text| serde_json::from_str::<SpawnRequest>(&text).ok());
        let Some(request) = request.filter(|r| !request_stale(r.at, now_secs())) else {
            fs::remove_file(&files.spawn).ok();
            return;
        };
        if locked(&self.workers).contains_key(id) {
            self.fail(id, "worker is already running");
            return;
        }
        let launch = Launch::Fresh(SessionRecord {
            session_id: String::new(),
            cwd: request.worktree.clone(),
            inbox: request.inbox.clone(),
            store: Some(self.store_dir.clone()),
            model: request.model.clone(),
            effort: None,
            permissions: None,
        });
        match self.launch(id, &request, launch) {
            Ok(worker) => self.adopt(id, worker),
            Err(message) => {
                fs::remove_file(&files.session).ok();
                self.fail(id, &message);
            }
        }
    }

    fn launch(
        &self,
        id: &str,
        request: &SpawnRequest,
        launch: Launch,
    ) -> Result<CodexWorker, String> {
        let files = self.files(id);
        let warning = matches!(launch, Launch::Fresh(_)).then_some(files.warning.as_path());
        let started = CodexWorker::start(
            request,
            Slot {
                id,
                owner: &files.owner,
                session: &files.session,
                results: &files.interrupt_result,
                launch,
                markers: WorkerMarkers::new(&self.store_dir, id),
                starting: &self.starting,
                warning,
            },
        );
        locked(&self.starting).remove(id);
        started
    }

    fn adopt(&self, id: &str, worker: CodexWorker) {
        let files = self.files(id);
        let claimed = !self.closed.load(Ordering::SeqCst)
            && fs::remove_file(&files.spawn).is_ok()
            && write_file(&ready_path(&self.store_dir, id), "").is_ok();
        if claimed {
            locked(&self.workers).insert(id.to_string(), worker);
        } else {
            drop(worker);
            fs::remove_file(ready_path(&self.store_dir, id)).ok();
            fs::remove_file(&files.session).ok();
        }
    }

    fn fail(&self, id: &str, message: &str) {
        let files = self.files(id);
        write_file(&files.error, message).ok();
        fs::remove_file(&files.spawn).ok();
    }
}

impl HeadlessHost for CodexHost {
    fn resume(&self) {
        self.resume_all();
    }

    fn poll(&self) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        CodexHost::poll(self);
        true
    }

    fn shutdown(&self) {
        CodexHost::shutdown(self);
    }
}
