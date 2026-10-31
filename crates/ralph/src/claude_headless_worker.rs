use crate::acp_log::{drain_stderr, CappedLog};
use crate::acp_worker::{apply_effect, push_event, Events};
use crate::codex_worker::pipes::{logged_stdin, pipe_lines};
use crate::codex_worker::{report_exit, Stdin};
use crate::config::{configured_permission_mode, AppConfig};
use crate::cursor_owner::{
    now_secs, queued_entries, report_start_warning, request_ids, sweep_owner, take_answer,
    write_file, write_interrupt_result, write_owner_record,
};
use crate::permission_queue::{host_process_token, PermissionQueue};
use crate::process_tree::{snapshot_tree, sweep_tree};
use crate::transport::stdio::{spawn_piped, stop_child};
use crate::watches::locked;
use crate::worker_mcp::Inbox;
use crate::workers::headless::{HeadlessHost, Heartbeat};
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::stream::{interrupt_line, user_line};
use ralph_core::worker::{
    acp_mcp_server, activity_path, auto_answer, bind_started_turn, check_agent_version,
    claude_headless_args, decode_claude_version, decode_line, gone_path, interrupted_end,
    permission_response, prompt_failure_report, ready_path, reduce, request_stale,
    stuck_permission_report, task_prompt, Action, ClaudeLine, ClaudePermission, HandoffFiles,
    HarnessId, HeadlessState, PermissionAnswer, PermissionMode, SpawnRequest, ThreadSignal, Turns,
    VerifiedAgents, WorkerEvent, WorkerMarkers, ATTACHMENTS_DIR, CLAUDE_COMMAND, CLAUDE_GATE,
    CLAUDE_HEADLESS_ANSWER_DIR, CLAUDE_HEADLESS_CLOSE_DIR, CLAUDE_HEADLESS_INTERRUPT_DIR,
    CLAUDE_HEADLESS_OWNER_DIR, CLAUDE_HEADLESS_PROMPT_DIR, CLAUDE_HEADLESS_SPAWN_DIR, SERVER_NAME,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

struct ClaudeReply {
    request_id: String,
    input: Value,
}

struct Shared {
    id: String,
    turns: Mutex<Turns>,
    stdin: Stdin,
    events: Events,
    permissions: PermissionQueue<ClaudeReply>,
    mode: PermissionMode,
    heartbeat: Arc<Heartbeat>,
    markers: WorkerMarkers,
    inbox: Inbox,
    results: PathBuf,
    turn_seq: AtomicU64,
    closing: AtomicBool,
    exited: AtomicBool,
}

impl Shared {
    fn drive(&self, step: impl FnOnce(&mut Turns) -> Vec<Action>) {
        let mut turns = locked(&self.turns);
        let actions = step(&mut turns);
        let seq = self.turn_seq.fetch_add(1, Ordering::SeqCst);
        let actions = bind_started_turn(&mut turns, actions, seq);
        self.perform(actions);
    }

    fn accept(&self, text: String) {
        self.drive(|turns| turns.accept(text));
    }

    fn interrupt(&self, text: String, entry: String) {
        self.drive(|turns| turns.interrupt(text, entry));
    }

    fn signal(&self, signal: ThreadSignal) {
        let closing = self.closing.load(Ordering::SeqCst);
        self.drive(|turns| {
            let signal = match signal {
                ThreadSignal::TurnEnded(end) => {
                    ThreadSignal::TurnEnded(interrupted_end(end, turns.interrupt_sent()))
                }
                other => other,
            };
            turns.signal(signal, closing)
        });
    }

    fn send(&self, line: &str) -> Result<(), String> {
        let mut stdin = locked(&self.stdin);
        let stdin = stdin.as_mut().ok_or("claude stdin is closed")?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("could not write to claude: {e}"))
    }

    fn ask(&self, ask: ClaudePermission) {
        match auto_answer(self.mode, &ask) {
            Some(line) => {
                self.send(&line).ok();
            }
            None => {
                let reply = ClaudeReply {
                    request_id: ask.content.request_id.clone(),
                    input: ask.input,
                };
                self.permissions.enqueue(reply, ask.content);
            }
        }
    }

    fn answer(&self, text: &str) -> Result<(), String> {
        let answer = PermissionAnswer::decode(text)?;
        let (reply, option) = self
            .permissions
            .take_answer(&answer.request_id, &answer.option_id)?;
        self.send(&permission_response(
            &reply.request_id,
            &option.option_id,
            &reply.input,
        ))
    }

    fn perform(&self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Start(text) => {
                    self.send(&user_line(&text)).ok();
                }
                Action::Emit(event) => {
                    match &event {
                        WorkerEvent::TurnStarted => self.heartbeat.set_running(true),
                        WorkerEvent::TurnEnded(_) => self.heartbeat.set_running(false),
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
                    let agent = HarnessId::ClaudeHeadless.as_str();
                    let report = prompt_failure_report(agent, &self.id, &at, &problem);
                    self.inbox.post(&report, 0).ok();
                }
                Action::Interrupt(turn_id) => {
                    self.send(&interrupt_line(&format!("interrupt-{turn_id}")))
                        .ok();
                    self.permissions.drain();
                }
                Action::InterruptResult { entry, cancelled } => {
                    write_interrupt_result(&self.results, &entry, cancelled);
                }
                Action::Snapshot => {}
            }
        }
    }

    fn read(&self, lines: mpsc::Receiver<String>, ready: mpsc::Sender<Result<(), String>>) {
        let mut ready = Some(ready);
        for line in lines {
            self.heartbeat.touch();
            match decode_line(&line) {
                ClaudeLine::Init(status) => {
                    if let Some(ready) = ready.take() {
                        ready.send(status).ok();
                    }
                }
                ClaudeLine::Signal(signal) => self.signal(signal),
                ClaudeLine::Permission(ask) => self.ask(ask),
                ClaudeLine::Cancel(id) => self.permissions.settle(|reply| reply.request_id == id),
                ClaudeLine::Malformed => {
                    let at = chrono::Utc::now().to_rfc3339();
                    let report = stuck_permission_report(&self.id, &at);
                    self.inbox.post(&report, 0).ok();
                }
                ClaudeLine::Other => {}
            }
        }
        if let Some(ready) = ready {
            ready
                .send(Err("claude exited before it was ready".to_string()))
                .ok();
        }
        report_exit(
            &self.heartbeat,
            &self.events,
            &self.exited,
            &self.closing,
            "claude exited",
        );
    }
}

fn check_installed_version(warning: &Path) -> Result<(), String> {
    let output = Command::new(CLAUDE_COMMAND)
        .arg("--version")
        .output()
        .map_err(|e| format!("could not run `{CLAUDE_COMMAND} --version`: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let found = decode_claude_version(&text).unwrap_or_default();
    if let Some(message) = check_agent_version(&CLAUDE_GATE, found, &VerifiedAgents::default())? {
        report_start_warning(message, Some(warning));
    }
    Ok(())
}

struct ClaudeWorker {
    child: Child,
    shared: Arc<Shared>,
    finished: mpsc::Receiver<()>,
    owner: PathBuf,
    state: HeadlessState,
}

impl ClaudeWorker {
    fn start(id: &str, request: &SpawnRequest, files: &HandoffFiles) -> Result<Self, String> {
        check_installed_version(&files.warning)?;
        let store = request.inbox.parent().unwrap_or(Path::new("."));
        let server = acp_mcp_server(
            &request.exe,
            id,
            &request.inbox,
            &files.owner,
            request.snapshots.then_some(request.worktree.as_path()),
        );
        let mcp = json!({"mcpServers": {SERVER_NAME: {"command": server["command"], "args": server["args"]}}});
        let attachments = store.join(ATTACHMENTS_DIR);
        let mode =
            configured_permission_mode("claude", |config| config.claude.permissions_setting());
        let args = claude_headless_args(
            request.model.as_deref(),
            &mcp.to_string(),
            &attachments,
            mode,
        );
        let mut command = Command::new(CLAUDE_COMMAND);
        command.args(args).current_dir(&request.worktree);
        let config_dir = AppConfig::load().ok().and_then(|config| {
            configured_dir(
                config.claude.config_dir.as_deref(),
                dirs::home_dir().as_deref(),
            )
        });
        if let Some(dir) = config_dir {
            command.env("CLAUDE_CONFIG_DIR", dir);
        }
        let (mut child, stdin, stdout, stderr) =
            spawn_piped(command).map_err(|e| format!("could not start `{CLAUDE_COMMAND}`: {e}"))?;
        let log = CappedLog::new(Some(store), id);
        drain_stderr(stderr, Arc::clone(&log));
        let markers = WorkerMarkers::new(store, id);
        let heartbeat = Heartbeat::new(markers.activity.clone(), markers.gone.clone());
        heartbeat.clear();
        let (lines, finished) = pipe_lines(stdout, &log);
        let events = Events::default();
        let shared = Arc::new(Shared {
            id: id.to_string(),
            turns: Mutex::new(Turns::default()),
            stdin: logged_stdin(stdin, &log),
            permissions: PermissionQueue::new(
                host_process_token(),
                id.to_string(),
                Inbox::new(request.inbox.clone()),
                Arc::clone(&events),
            ),
            mode,
            events,
            heartbeat,
            markers,
            inbox: Inbox::new(request.inbox.clone()),
            results: files.interrupt_result.clone(),
            turn_seq: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });
        if let Err(message) = write_owner_record(&files.owner, child.id(), "claude") {
            stop_child(&mut child);
            return Err(message);
        }
        let mut worker = Self {
            child,
            shared: Arc::clone(&shared),
            finished,
            owner: files.owner.clone(),
            state: HeadlessState::default(),
        };
        let (ready_tx, ready) = mpsc::channel();
        let reader = Arc::clone(&shared);
        thread::spawn(move || reader.read(lines, ready_tx));
        shared.accept(task_prompt(&request.task));
        let ready = ready.recv_timeout(READY_TIMEOUT).unwrap_or_else(|_| {
            Err(format!(
                "claude was not ready after {}s",
                READY_TIMEOUT.as_secs()
            ))
        });
        ready.inspect_err(|_| worker.terminate()).map(|()| worker)
    }

    fn apply_events(&mut self) {
        let events = std::mem::take(&mut *locked(&self.shared.events));
        for event in events {
            let (state, effects) = reduce(&self.state, &event);
            self.state = state;
            for effect in &effects {
                apply_effect(&self.shared.markers, effect);
            }
        }
    }

    fn terminate(&mut self) {
        if self.shared.closing.swap(true, Ordering::SeqCst) {
            return;
        }
        self.shared.permissions.drain();
        let tree = snapshot_tree(self.child.id());
        locked(&self.shared.stdin).take();
        self.finished.recv_timeout(SHUTDOWN_GRACE).ok();
        stop_child(&mut self.child);
        sweep_tree(&tree);
        fs::remove_file(&self.owner).ok();
    }
}

impl Drop for ClaudeWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub(crate) struct ClaudeHeadlessHost {
    store_dir: PathBuf,
    workers: Mutex<HashMap<String, ClaudeWorker>>,
    closed: AtomicBool,
}

impl ClaudeHeadlessHost {
    pub(crate) fn new(store_dir: PathBuf) -> Arc<Self> {
        for id in request_ids(&store_dir, CLAUDE_HEADLESS_OWNER_DIR) {
            sweep_owner(&HandoffFiles::claude_headless(&store_dir, &id));
        }
        Arc::new(Self {
            store_dir,
            workers: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    fn files(&self, id: &str) -> HandoffFiles {
        HandoffFiles::claude_headless(&self.store_dir, id)
    }

    fn deliver_queued(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&Shared, String, String),
    ) {
        for entry in queued_entries(dir(&self.files(id))) {
            let name = entry
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let worker = locked(&self.workers).get(id).map(|w| Arc::clone(&w.shared));
            if let (Ok(text), Some(shared)) = (fs::read_to_string(&entry), worker) {
                deliver(&shared, text, name);
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn answer(&self, id: &str) {
        take_answer(&self.files(id), |text| {
            let worker = locked(&self.workers).get(id).map(|w| Arc::clone(&w.shared));
            worker
                .ok_or_else(|| "worker has no running claude process".to_string())
                .and_then(|shared| shared.answer(text))
        });
    }

    fn close(&self, id: &str) {
        let removed = locked(&self.workers).remove(id);
        if let Some(mut worker) = removed {
            worker.terminate();
            push_event(&worker.shared.events, WorkerEvent::Closed);
            worker.apply_events();
        }
        self.forget(id);
        let files = self.files(id);
        fs::remove_file(ready_path(&self.store_dir, id)).ok();
        fs::remove_file(&files.close).ok();
    }

    fn forget(&self, id: &str) {
        let files = self.files(id);
        fs::remove_file(&files.owner).ok();
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_file(activity_path(&self.store_dir, id)).ok();
        fs::remove_file(gone_path(&self.store_dir, id)).ok();
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
        match ClaudeWorker::start(id, &request, &files) {
            Ok(worker) => self.adopt(id, worker),
            Err(message) => self.fail(id, &message),
        }
    }

    fn adopt(&self, id: &str, worker: ClaudeWorker) {
        let files = self.files(id);
        let claimed = !self.closed.load(Ordering::SeqCst)
            && fs::remove_file(&files.spawn).is_ok()
            && write_file(&ready_path(&self.store_dir, id), "").is_ok();
        if claimed {
            locked(&self.workers).insert(id.to_string(), worker);
        } else {
            drop(worker);
            fs::remove_file(ready_path(&self.store_dir, id)).ok();
        }
    }

    fn fail(&self, id: &str, message: &str) {
        let files = self.files(id);
        write_file(&files.error, message).ok();
        fs::remove_file(&files.spawn).ok();
    }
}

impl HeadlessHost for ClaudeHeadlessHost {
    fn resume(&self) {}

    fn poll(&self) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        for worker in locked(&self.workers).values_mut() {
            worker.apply_events();
        }
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_CLOSE_DIR) {
            self.close(&id);
        }
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_ANSWER_DIR) {
            self.answer(&id);
        }
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_PROMPT_DIR) {
            self.deliver_queued(
                &id,
                |files| &files.prompt,
                |shared, text, _| shared.accept(text),
            );
        }
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_INTERRUPT_DIR) {
            self.deliver_queued(
                &id,
                |files| &files.interrupt,
                |shared, text, entry| shared.interrupt(text, entry),
            );
        }
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_SPAWN_DIR) {
            self.spawn(&id);
        }
        true
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let drained: Vec<(String, ClaudeWorker)> = locked(&self.workers).drain().collect();
        for (id, worker) in drained {
            drop(worker);
            self.forget(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::Report;
    use tempfile::TempDir;

    const ASK: &str = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"t1"}}"#;

    struct Run {
        shared: Shared,
        dir: TempDir,
        events: Vec<WorkerEvent>,
    }

    impl Run {
        fn reports(&self) -> Vec<Report> {
            let Ok(entries) = fs::read_dir(self.dir.path().join("inbox")) else {
                return Vec::new();
            };
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| fs::read(entry.path()).ok())
                .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
                .collect()
        }

        fn permission_events(&self) -> Vec<&WorkerEvent> {
            self.events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        WorkerEvent::PermissionAsked(_) | WorkerEvent::PermissionSettled
                    )
                })
                .collect()
        }
    }

    fn run(mode: PermissionMode, lines: &[&str]) -> Run {
        let dir = TempDir::new().unwrap();
        let store = dir.path();
        let events = Events::default();
        let inbox = || Inbox::new(store.join("inbox"));
        let shared = Shared {
            id: "w".into(),
            turns: Mutex::default(),
            stdin: Arc::new(Mutex::new(None)),
            events: Arc::clone(&events),
            permissions: PermissionQueue::new("p".into(), "w".into(), inbox(), Arc::clone(&events)),
            mode,
            heartbeat: Heartbeat::new(store.join("activity"), store.join("gone")),
            markers: WorkerMarkers::new(store, "w"),
            inbox: inbox(),
            closing: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            results: store.join("interrupt-result"),
            turn_seq: AtomicU64::new(0),
        };
        let (tx, rx) = mpsc::channel();
        for line in lines {
            tx.send((*line).to_string()).unwrap();
        }
        drop(tx);
        let (ready, _) = mpsc::channel();
        shared.read(rx, ready);
        let events = locked(&events).clone();
        Run {
            shared,
            dir,
            events,
        }
    }

    #[test]
    fn a_permission_line_without_a_request_id_posts_one_failure_and_queues_nothing() {
        let line = ASK.replace(r#""request_id":"r1","#, "");
        let run = run(PermissionMode::Ask, &[&line]);
        assert!(run.permission_events().is_empty());
        let reports = run.reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, "blocked");
        assert_eq!(
            reports[0].text,
            "claude sent a permission request ralph can't answer (no request_id or no input object); the worker is stuck — close it."
        );
    }

    // stdin is None here; core test yolo_answers_allow_once_and_ask_answers_nothing covers the bytes.
    #[test]
    fn yolo_never_queues_or_publishes() {
        let run = run(PermissionMode::Yolo, &[ASK]);
        assert!(run.permission_events().is_empty());
        assert!(run.reports().is_empty());
    }

    #[test]
    fn a_cancel_settles_by_the_raw_id_after_the_queue_published_the_prefixed_id() {
        let cancel = r#"{"type":"control_cancel_request","request_id":"r1"}"#;
        let run = run(PermissionMode::Ask, &[ASK, cancel]);
        let asked = run.permission_events();
        assert!(matches!(
            asked.as_slice(),
            [WorkerEvent::PermissionAsked(content), WorkerEvent::PermissionSettled]
                if content.request_id == "p.r1"
        ));
        let stale = run
            .shared
            .permissions
            .take_answer("p.r1", "allow_once")
            .err()
            .unwrap();
        assert!(stale.starts_with("stale request"));
    }
}
