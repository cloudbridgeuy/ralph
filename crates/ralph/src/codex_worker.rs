use crate::acp_log::{drain_stderr, CappedLog, LoggedRead, LoggedWrite};
use crate::acp_worker::{apply_effect, push_event, Events};
use crate::cursor_owner::{
    now_secs, queued_entries, request_ids, sweep_owner, write_file, write_owner_record,
};
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::warn::warn;
use crate::worker_changes::WorkerChanges;
use crate::worker_mcp::Inbox;
use crate::workers::headless::HeadlessHost;
use ralph_core::snapshot::WorkerSite;
use ralph_core::worker::{
    acp_mcp_server, activity_path, check_login, check_model, check_version, decide, decode_signal,
    decode_thread_id, decode_version, describe, gone_path, initialize_params, model_ids,
    model_list_params, prompt_failure_report, ready_path, reduce, reject_server_request,
    render_gone, request_stale, task_prompt, thread_start_params, turn_start_params, Action,
    Capabilities, HandoffFiles, HarnessId, HeadlessState, HeadlessWorker, Incoming, McpStartup,
    McpVerdict, Rpc, Scope, SessionRecord, SpawnRequest, ThreadSignal, TurnEnd, Turns,
    WorkerCommand, WorkerEvent, WorkerMarkers, ACCOUNT_READ, CODEX_CLOSE_DIR, CODEX_COMMAND,
    CODEX_OWNER_DIR, CODEX_PROMPT_DIR, CODEX_SPAWN_DIR, INITIALIZE, INITIALIZED, MODEL_LIST,
    SERVER_NAME, THREAD_START, TURN_START,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
const REPLY_POLL: Duration = Duration::from_millis(100);
const EXIT_REASON: &str = "codex app-server exited";

type Stdin = Arc<Mutex<Option<LoggedWrite<ChildStdin>>>>;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Stop {
    Timeout,
    Gone,
    Bad(String),
}

impl Stop {
    fn message(self) -> String {
        match self {
            Self::Timeout => format!("codex was not ready after {}s", READY_TIMEOUT.as_secs()),
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
}

impl Wire {
    fn send_line(&self, line: &str) -> Result<(), Stop> {
        let mut guard = locked(&self.stdin);
        let stdin = guard.as_mut().ok_or(Stop::Gone)?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|e| Stop::Bad(format!("could not write to codex: {e}")))
    }

    fn next(&mut self, deadline: Option<Instant>) -> Result<Incoming, Stop> {
        loop {
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
            match self
                .rpc
                .receive(&line)
                .map_err(|e| Stop::Bad(e.to_string()))?
            {
                Incoming::ServerRequest { id, method, .. } => {
                    self.send_line(&reject_server_request(&id, &method))?;
                }
                incoming => {
                    if let Incoming::Notification { method, params } = &incoming {
                        self.startup.observe(method, params);
                    }
                    return Ok(incoming);
                }
            }
        }
    }

    fn call(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, String> {
        let (_, line) = self.rpc.request(method, params);
        self.send_line(&line).map_err(Stop::message)?;
        loop {
            if let Incoming::Response { outcome, .. } =
                self.next(Some(deadline)).map_err(Stop::message)?
            {
                return outcome.map_err(|e| format!("codex {method} failed: {}", e.message));
            }
        }
    }
}

fn handshake(
    wire: &mut Wire,
    request: &SpawnRequest,
    server: &Value,
    deadline: Instant,
) -> Result<String, String> {
    let init = wire.call(
        INITIALIZE,
        initialize_params(env!("CARGO_PKG_VERSION")),
        deadline,
    )?;
    wire.send_line(&Rpc::notification(INITIALIZED))
        .map_err(Stop::message)?;
    let version = decode_version(&init)
        .ok_or("could not read the codex version from its initialize reply")?;
    if let Some(warning) = check_version(&version)? {
        warn(warning);
    }
    check_login(&wire.call(ACCOUNT_READ, json!({}), deadline)?)?;
    if request.model.is_some() {
        let listed = wire.call(MODEL_LIST, model_list_params(), deadline)?;
        check_model(request.model.as_deref(), &model_ids(&listed))?;
    }
    let started = wire.call(
        THREAD_START,
        thread_start_params(&request.worktree, request.model.as_deref(), server),
        deadline,
    )?;
    let thread = decode_thread_id(&started).ok_or("codex did not return a thread id")?;
    loop {
        match wire.startup.verdict(&[SERVER_NAME]) {
            McpVerdict::Ready => return Ok(thread),
            McpVerdict::Failed { server, detail } => {
                return Err(format!(
                    "codex could not start the {server} MCP server: {detail}"
                ))
            }
            McpVerdict::Waiting => {
                wire.next(Some(deadline)).map_err(Stop::message)?;
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
    replies: mpsc::Receiver<String>,
    inbox: Inbox,
    changes: Option<WorkerChanges>,
    id: String,
}

impl Pump {
    fn perform(&mut self, actions: Vec<Action>) -> Result<(), String> {
        for action in actions {
            match action {
                Action::Start(text) => {
                    let params = turn_start_params(&self.thread, &text);
                    self.wire
                        .call(TURN_START, params, Instant::now() + READY_TIMEOUT)?;
                }
                Action::Emit(event) => push_event(&self.events, event),
                Action::MarkUnreported => {
                    write_file(&self.markers.unreported, "").ok();
                    write_file(&self.markers.waiting, "").ok();
                }
                Action::Snapshot => {
                    if let Some(changes) = &self.changes {
                        changes.snapshot_turn_or_warn();
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

    fn settle(&mut self, signal: ThreadSignal) {
        let mut actions = self
            .turns
            .signal(signal, self.closing.load(Ordering::SeqCst));
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

    fn run(mut self) {
        loop {
            while let Ok(text) = self.replies.try_recv() {
                let actions = self.turns.accept(text);
                if let Err(message) = self.perform(actions) {
                    self.settle(ThreadSignal::TurnEnded(TurnEnd::Failed {
                        message,
                        transient: false,
                    }));
                }
            }
            match self.wire.next(Some(Instant::now() + REPLY_POLL)) {
                Err(Stop::Gone) => return,
                Ok(Incoming::Notification { method, params }) => {
                    if let Some(signal) = decode_signal(&method, &params) {
                        self.settle(signal);
                    }
                }
                _ => {}
            }
        }
    }
}

pub struct Slot<'a> {
    pub id: &'a str,
    pub owner: &'a Path,
    pub session: &'a Path,
    pub template: SessionRecord,
    pub markers: WorkerMarkers,
    pub starting: &'a Mutex<HashMap<String, u32>>,
}

pub struct CodexWorker {
    child: Child,
    stdin: Stdin,
    finished: mpsc::Receiver<()>,
    owner: PathBuf,
    closing: Arc<AtomicBool>,
    done: bool,
    caps: Capabilities,
    markers: WorkerMarkers,
    state: HeadlessState,
    events: Events,
    replies: mpsc::Sender<String>,
}

impl CodexWorker {
    pub fn start(request: &SpawnRequest, slot: Slot) -> Result<Self, String> {
        let Slot {
            id,
            owner,
            session,
            template,
            markers,
            starting,
        } = slot;
        let mut command = Command::new(CODEX_COMMAND);
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(&request.worktree);
        let (child, stdin, stdout, stderr) = spawn_piped(command)
            .map_err(|e| format!("could not start `{CODEX_COMMAND} app-server`: {e}"))?;
        let log = CappedLog::new(request.inbox.parent(), id);
        drain_stderr(stderr, Arc::clone(&log));
        let stdin: Stdin = Arc::new(Mutex::new(Some(LoggedWrite {
            inner: stdin,
            log: Arc::clone(&log),
            tag: "in",
        })));
        let (lines_tx, lines) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        thread::spawn(move || {
            let reader = BufReader::new(LoggedRead {
                inner: stdout,
                log,
                tag: "out",
            });
            for line in reader.lines().map_while(Result::ok) {
                if lines_tx.send(line).is_err() {
                    break;
                }
            }
            finished_tx.send(()).ok();
        });
        fs::remove_file(&markers.activity).ok();
        fs::remove_file(&markers.gone).ok();
        let closing = Arc::new(AtomicBool::new(false));
        let (replies, reply_rx) = mpsc::channel();
        let worker = Self {
            child,
            stdin: Arc::clone(&stdin),
            finished,
            owner: owner.to_path_buf(),
            closing: Arc::clone(&closing),
            done: false,
            caps: ralph_core::worker::capabilities(HarnessId::Codex),
            markers: markers.clone(),
            state: HeadlessState::default(),
            events: Events::default(),
            replies,
        };
        write_owner_record(owner, worker.child.id(), "codex")?;
        locked(starting).insert(id.to_string(), worker.child.id());
        let mut wire = Wire {
            rpc: Rpc::default(),
            stdin,
            lines,
            startup: McpStartup::default(),
        };
        let server = acp_mcp_server(
            &request.exe,
            id,
            &request.inbox,
            owner,
            request.snapshots.then_some(request.worktree.as_path()),
        );
        let deadline = Instant::now() + READY_TIMEOUT;
        let thread_id = handshake(&mut wire, request, &server, deadline)?;
        let record = SessionRecord {
            session_id: thread_id.clone(),
            ..template
        };
        write_file(session, &record.render())
            .map_err(|e| format!("could not write the codex session file: {e}"))?;
        let mut pump = Pump {
            wire,
            thread: thread_id,
            turns: Turns::default(),
            events: Arc::clone(&worker.events),
            markers: markers.clone(),
            closing,
            replies: reply_rx,
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
        };
        let first = pump.turns.accept(task_prompt(&request.task));
        pump.perform(first)?;
        let gone = markers.gone;
        let closing = Arc::clone(&pump.closing);
        thread::spawn(move || {
            pump.run();
            if !closing.load(Ordering::SeqCst) {
                write_file(&gone, &render_gone(&Scope::Worker, EXIT_REASON)).ok();
            }
        });
        Ok(worker)
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

    fn begin_stop(&mut self) {
        self.done = true;
        self.closing.store(true, Ordering::SeqCst);
        locked(&self.stdin).take();
    }

    fn finish_stop(&mut self, deadline: Instant) {
        self.finished
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok();
        stop_child(&mut self.child);
        fs::remove_file(&self.owner).ok();
    }

    fn terminate(&mut self) {
        if self.done {
            return;
        }
        self.begin_stop();
        self.finish_stop(Instant::now() + SHUTDOWN_GRACE);
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
                .send(text)
                .map_err(|_| "codex worker is not running".to_string()),
            WorkerCommand::Spawn(_)
            | WorkerCommand::Resume
            | WorkerCommand::Interrupt(_)
            | WorkerCommand::AnswerPermission { .. } => Err(format!(
                "not sent: the host handles it ({})",
                describe(&self.caps, command.feature())
            )),
        }
    }

    fn poll(&mut self) -> Vec<WorkerEvent> {
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
        for id in request_ids(&self.store_dir, CODEX_PROMPT_DIR) {
            self.deliver_prompts(&id);
        }
        for id in request_ids(&self.store_dir, CODEX_SPAWN_DIR) {
            self.spawn(&id);
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut drained: Vec<(String, CodexWorker)> = locked(&self.workers).drain().collect();
        for (_, worker) in &mut drained {
            worker.begin_stop();
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        for (id, worker) in &mut drained {
            worker.finish_stop(deadline);
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

    fn deliver_prompts(&self, id: &str) {
        let starting = locked(&self.starting).contains_key(id);
        if starting && !locked(&self.workers).contains_key(id) {
            return;
        }
        for entry in queued_entries(&self.files(id).prompt) {
            if let (Ok(text), Some(worker)) = (
                fs::read_to_string(&entry),
                locked(&self.workers).get_mut(id),
            ) {
                if let Err(message) = worker.send(WorkerCommand::Prompt(text)) {
                    warn(message);
                }
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn forget(&self, id: &str) {
        let files = self.files(id);
        fs::remove_file(&files.owner).ok();
        fs::remove_dir_all(&files.prompt).ok();
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
        let started = CodexWorker::start(
            &request,
            Slot {
                id,
                owner: &files.owner,
                session: &files.session,
                template: SessionRecord {
                    session_id: String::new(),
                    cwd: request.worktree.clone(),
                    inbox: request.inbox.clone(),
                    store: Some(self.store_dir.clone()),
                    model: request.model.clone(),
                },
                markers: WorkerMarkers::new(&self.store_dir, id),
                starting: &self.starting,
            },
        );
        locked(&self.starting).remove(id);
        match started {
            Ok(worker) => self.adopt(id, worker),
            Err(message) => {
                fs::remove_file(&files.session).ok();
                self.fail(id, &message);
            }
        }
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
    fn resume(&self) {}

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
