use crate::acp_log::{drain_stderr, CappedLog};
use crate::acp_worker::{apply_effect, push_event, Events};
use crate::codex_worker::pipes::{logged_stdin, pipe_lines};
use crate::codex_worker::{report_exit, Stdin};
use crate::config::AppConfig;
use crate::cursor_owner::{
    now_secs, queued_entries, report_start_warning, request_ids, sweep_owner, write_file,
    write_owner_record,
};
use crate::process_tree::{snapshot_tree, sweep_tree};
use crate::transport::stdio::{spawn_piped, stop_child};
use crate::watches::locked;
use crate::worker_mcp::Inbox;
use crate::workers::headless::{HeadlessHost, Heartbeat};
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::stream::user_line;
use ralph_core::worker::{
    acp_mcp_server, activity_path, check_agent_version, claude_headless_args,
    decode_claude_version, decode_line, gone_path, prompt_failure_report, ready_path, reduce,
    request_stale, task_prompt, Action, ClaudeLine, HandoffFiles, HarnessId, HeadlessState,
    SpawnRequest, Turns, VerifiedAgents, WorkerEvent, WorkerMarkers, ATTACHMENTS_DIR,
    CLAUDE_COMMAND, CLAUDE_GATE, CLAUDE_HEADLESS_CLOSE_DIR, CLAUDE_HEADLESS_OWNER_DIR,
    CLAUDE_HEADLESS_PROMPT_DIR, CLAUDE_HEADLESS_SPAWN_DIR, SERVER_NAME,
};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

struct Shared {
    id: String,
    turns: Mutex<Turns>,
    stdin: Stdin,
    events: Events,
    heartbeat: Arc<Heartbeat>,
    markers: WorkerMarkers,
    inbox: Inbox,
    closing: AtomicBool,
    exited: AtomicBool,
}

impl Shared {
    fn accept(&self, text: String) {
        let mut turns = locked(&self.turns);
        let actions = turns.accept(text);
        self.perform(actions);
    }

    fn perform(&self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Start(text) => {
                    if let Some(stdin) = locked(&self.stdin).as_mut() {
                        writeln!(stdin, "{}", user_line(&text))
                            .and_then(|()| stdin.flush())
                            .ok();
                    }
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
                Action::Interrupt(_) | Action::InterruptResult { .. } | Action::Snapshot => {}
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
                ClaudeLine::Signal(signal) => {
                    let mut turns = locked(&self.turns);
                    let actions = turns.signal(signal, self.closing.load(Ordering::SeqCst));
                    self.perform(actions);
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
        let args = claude_headless_args(request.model.as_deref(), &mcp.to_string(), &attachments);
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
        let shared = Arc::new(Shared {
            id: id.to_string(),
            turns: Mutex::new(Turns::default()),
            stdin: logged_stdin(stdin, &log),
            events: Events::default(),
            heartbeat,
            markers,
            inbox: Inbox::new(request.inbox.clone()),
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

    fn deliver_prompts(&self, id: &str) {
        for entry in queued_entries(&self.files(id).prompt) {
            let worker = locked(&self.workers).get(id).map(|w| Arc::clone(&w.shared));
            if let (Ok(text), Some(shared)) = (fs::read_to_string(&entry), worker) {
                shared.accept(text);
            }
            fs::remove_file(&entry).ok();
        }
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
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_PROMPT_DIR) {
            self.deliver_prompts(&id);
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
