use crate::acp_log::{drain_stderr, CappedLog, LoggedWrite};
use crate::acp_worker::{apply_effect, push_event, Events};
use crate::codex_worker::pipes::pipe_lines;
use crate::codex_worker::report_exit;
use crate::codex_worker::resume::Resumed;
use crate::config::{configured_permission_mode, AppConfig};
use crate::cursor_owner::{
    now_secs, queued_entries, report_start_warning, request_ids, sweep_owner, take_answer,
    write_file, write_interrupt_result, write_owner_record,
};
use crate::permission_queue::{host_process_token, PermissionQueue};
use crate::process_tree::{snapshot_tree, sweep_tree};
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::watches::locked;
use crate::worker_mcp::Inbox;
use crate::workers::headless::{HeadlessHost, Heartbeat};
use ralph_core::claude_leftovers::configured_dir;
use ralph_core::stream::user_line;
use ralph_core::worker::{
    acp_mcp_server, activity_path, auto_answer, bind_started_turn, check_agent_version,
    claude_headless_args, cleanup, decode_claude_version, decode_line, deny_line, gone_path,
    interrupt_lines, interrupted_end, permission_response, prompt_failure_report, ready_path,
    reduce, request_stale, resumed_session, start_steps, stuck_permission_report, task_prompt,
    turn_lost_at_resume, uuid_v4, Action, ClaudeLine, ClaudePermission, ClaudeSession,
    HandoffFiles, HarnessId, HeadlessState, InterruptOutcome, Leftover, PermissionAnswer,
    PermissionMode, SessionRecord, SpawnRequest, StartFault, StartStep, ThreadSignal, Turns,
    VerifiedAgents, WorkerEvent, WorkerMarkers, ATTACHMENTS_DIR, CLAUDE_COMMAND, CLAUDE_GATE,
    CLAUDE_HEADLESS_ANSWER_DIR, CLAUDE_HEADLESS_CLOSE_DIR, CLAUDE_HEADLESS_INTERRUPT_DIR,
    CLAUDE_HEADLESS_OWNER_DIR, CLAUDE_HEADLESS_PROMPT_DIR, CLAUDE_HEADLESS_SESSION_DIR,
    CLAUDE_HEADLESS_SPAWN_DIR, SERVER_NAME,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
const LOST_TURN: &str = "the turn in flight was lost when the chat exited; prompt it again";

type ClaudeStdin = Mutex<Option<Box<dyn Write + Send>>>;

struct ClaudeReply {
    request_id: String,
    input: Value,
}

struct Shared {
    id: String,
    turns: Mutex<Turns>,
    stdin: ClaudeStdin,
    events: Events,
    permissions: PermissionQueue<ClaudeReply>,
    mode: PermissionMode,
    heartbeat: Arc<Heartbeat>,
    markers: WorkerMarkers,
    inbox: Inbox,
    results: PathBuf,
    expected_session: Option<String>,
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
                if let Some(refused) = self.permissions.enqueue_unless_refusing(reply, ask.content)
                {
                    self.send(&deny_line(&refused.request_id)).ok();
                }
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

    fn interrupt_sends(&self, turn_id: &str) -> Vec<String> {
        let pending = self
            .permissions
            .drain_and_refuse()
            .into_iter()
            .map(|reply| reply.request_id)
            .collect::<Vec<_>>();
        interrupt_lines(&pending, turn_id)
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
                        WorkerEvent::TurnEnded(_) => {
                            self.permissions.accept_again();
                            self.heartbeat.set_running(false);
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
                    let agent = HarnessId::ClaudeHeadless.as_str();
                    let report = prompt_failure_report(agent, &self.id, &at, &problem);
                    self.inbox.post(&report, 0).ok();
                }
                Action::Interrupt(turn_id) => {
                    for line in self.interrupt_sends(&turn_id) {
                        self.send(&line).ok();
                    }
                }
                Action::InterruptResult { entry, outcome } => {
                    write_interrupt_result(&self.results, &entry, outcome);
                }
                Action::Snapshot => {}
            }
        }
    }

    fn read(
        &self,
        lines: mpsc::Receiver<String>,
        mut ready: Option<mpsc::Sender<Result<String, String>>>,
    ) {
        for line in lines {
            self.heartbeat.touch();
            match decode_line(&line) {
                ClaudeLine::Init(status) => {
                    let status = status
                        .and_then(|found| resumed_session(self.expected_session.as_deref(), found));
                    if let Some(ready) = ready.take() {
                        ready.send(status).ok();
                    } else if let Err(problem) = status {
                        self.perform(vec![Action::PostFailure(problem)]);
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
        self.drive(Turns::process_gone);
        report_exit(
            &self.heartbeat,
            &self.events,
            &self.exited,
            &self.closing,
            "claude exited",
        );
    }
}

fn session_record(
    session_id: &str,
    request: &SpawnRequest,
    store: &Path,
    mode: PermissionMode,
) -> SessionRecord {
    SessionRecord {
        session_id: session_id.to_string(),
        cwd: request.worktree.clone(),
        inbox: request.inbox.clone(),
        store: Some(store.to_path_buf()),
        model: request.model.clone(),
        effort: None,
        permissions: Some(mode),
    }
}

fn remove_leftovers(leftovers: &[Leftover], files: &HandoffFiles, mut child: Option<&mut Child>) {
    for leftover in leftovers {
        match leftover {
            Leftover::StopChild => {
                if let Some(child) = child.as_deref_mut() {
                    stop_child(child);
                }
            }
            Leftover::RemoveOwner => {
                fs::remove_file(&files.owner).ok();
            }
            Leftover::RemoveRecord => {
                fs::remove_file(&files.session).ok();
            }
        }
    }
}

fn write_start_files(
    steps: &[StartStep],
    files: &HandoffFiles,
    record: &SessionRecord,
    group: u32,
    mut child: Option<&mut Child>,
) -> Result<(), String> {
    for step in steps {
        let (fault, written) = match step {
            StartStep::WriteOwner => (
                StartFault::OwnerWrite,
                write_owner_record(&files.owner, group, "claude"),
            ),
            StartStep::WriteRecord => (
                StartFault::RecordWrite,
                write_file(&files.session, &record.render())
                    .map_err(|e| format!("could not write the session record: {e}")),
            ),
        };
        if let Err(message) = written {
            remove_leftovers(cleanup(fault), files, child.as_deref_mut());
            return Err(message);
        }
    }
    Ok(())
}

fn random_bytes() -> Result<[u8; 16], String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| format!("could not read random bytes for the session id: {e}"))?;
    Ok(bytes)
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
    session: String,
}

impl ClaudeWorker {
    fn start(
        id: &str,
        request: &SpawnRequest,
        files: &HandoffFiles,
        resume: Option<&SessionRecord>,
        starting: &Mutex<HashMap<String, u32>>,
    ) -> Result<Self, String> {
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
        let mode = resume
            .and_then(|record| record.permissions)
            .unwrap_or_else(|| {
                configured_permission_mode("claude", |config| config.claude.permissions_setting())
            });
        let new_id;
        let session = match resume {
            Some(record) => ClaudeSession::Resume(&record.session_id),
            None => {
                new_id = uuid_v4(random_bytes()?);
                ClaudeSession::New(&new_id)
            }
        };
        let expected_session = match session {
            ClaudeSession::New(id) | ClaudeSession::Resume(id) => id.to_string(),
        };
        let args = claude_headless_args(
            request.model.as_deref(),
            &mcp.to_string(),
            &attachments,
            mode,
            session,
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
        locked(starting).insert(id.to_string(), child.id());
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
            stdin: Mutex::new(Some(Box::new(LoggedWrite {
                inner: stdin,
                log: Arc::clone(&log),
                tag: "in",
            }))),
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
            expected_session: Some(expected_session.clone()),
            turn_seq: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });
        let fresh;
        let record = match resume {
            Some(record) => record,
            None => {
                fresh = session_record(&expected_session, request, store, mode);
                &fresh
            }
        };
        write_start_files(
            start_steps(&session),
            files,
            record,
            child.id(),
            Some(&mut child),
        )?;
        let mut worker = Self {
            child,
            shared: Arc::clone(&shared),
            finished,
            owner: files.owner.clone(),
            state: HeadlessState::default(),
            session: String::new(),
        };
        let (ready_tx, ready) = mpsc::channel();
        let reader = Arc::clone(&shared);
        let waits = resume.is_none().then_some(ready_tx);
        thread::spawn(move || reader.read(lines, waits));
        let session = match resume {
            Some(record) => Ok(record.session_id.clone()),
            None => {
                shared.accept(task_prompt(&request.task));
                ready.recv_timeout(READY_TIMEOUT).unwrap_or_else(|_| {
                    Err(format!(
                        "claude was not ready after {}s",
                        READY_TIMEOUT.as_secs()
                    ))
                })
            }
        };
        let session = session.inspect_err(|_| {
            worker.terminate();
            remove_leftovers(cleanup(StartFault::InitFailed), files, None);
        })?;
        worker.session = session;
        Ok(worker)
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
    starting: Mutex<HashMap<String, u32>>,
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
            starting: Mutex::new(HashMap::new()),
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
        orphan: fn(&HandoffFiles, &str),
    ) {
        if locked(&self.starting).contains_key(id) && !locked(&self.workers).contains_key(id) {
            return;
        }
        let files = self.files(id);
        for entry in queued_entries(dir(&files)) {
            let name = entry
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let worker = locked(&self.workers).get(id).map(|w| Arc::clone(&w.shared));
            match (fs::read_to_string(&entry), worker) {
                (Ok(text), Some(shared)) => deliver(&shared, text, name),
                (_, None) => orphan(&files, &name),
                (Err(_), Some(_)) => {}
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
        fs::remove_file(&files.session).ok();
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
        match self.launch(id, &request, None) {
            Ok(worker) => self.adopt(id, worker, &request),
            Err(message) => self.fail(id, &message),
        }
    }

    fn launch(
        &self,
        id: &str,
        request: &SpawnRequest,
        resume: Option<&SessionRecord>,
    ) -> Result<ClaudeWorker, String> {
        let started = ClaudeWorker::start(id, request, &self.files(id), resume, &self.starting);
        locked(&self.starting).remove(id);
        started
    }

    fn adopt(&self, id: &str, worker: ClaudeWorker, request: &SpawnRequest) {
        let files = self.files(id);
        let claimed = !self.closed.load(Ordering::SeqCst)
            && fs::remove_file(&files.spawn).is_ok()
            && write_file(&ready_path(&self.store_dir, id), "").is_ok();
        if claimed {
            let record = session_record(
                &worker.session,
                request,
                &self.store_dir,
                worker.shared.mode,
            );
            write_file(&files.session, &record.render()).ok();
            locked(&self.workers).insert(id.to_string(), worker);
        } else {
            drop(worker);
            fs::remove_file(ready_path(&self.store_dir, id)).ok();
            remove_leftovers(cleanup(StartFault::NotAdopted), &files, None);
        }
    }

    fn fail(&self, id: &str, message: &str) {
        let files = self.files(id);
        write_file(&files.error, message).ok();
        fs::remove_file(&files.spawn).ok();
    }
}

impl HeadlessHost for ClaudeHeadlessHost {
    fn resume(&self) {
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_SESSION_DIR) {
            let files = self.files(&id);
            let activity = fs::read_to_string(activity_path(&self.store_dir, &id)).ok();
            let lost = turn_lost_at_resume(activity.as_deref());
            let resumed = Resumed {
                agent: HarnessId::ClaudeHeadless.as_str(),
                id: &id,
                store: &self.store_dir,
                files: &files,
            };
            let worker =
                resumed.launch(|record, request| self.launch(&id, &request, Some(&record)));
            if let Some(worker) = worker {
                if resumed.adopt(&self.closed) {
                    if lost {
                        worker
                            .shared
                            .perform(vec![Action::PostFailure(LOST_TURN.to_string())]);
                    }
                    locked(&self.workers).insert(id.clone(), worker);
                }
            }
        }
    }

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
                |_, _| {},
            );
        }
        for id in request_ids(&self.store_dir, CLAUDE_HEADLESS_INTERRUPT_DIR) {
            self.deliver_queued(
                &id,
                |files| &files.interrupt,
                |shared, text, entry| shared.interrupt(text, entry),
                |files, entry| {
                    write_interrupt_result(&files.interrupt_result, entry, InterruptOutcome::Gone);
                },
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
            if worker.shared.heartbeat.running() {
                worker
                    .shared
                    .perform(vec![Action::PostFailure(LOST_TURN.to_string())]);
            }
            drop(worker);
            self.forget(&id);
        }
        let starting: Vec<(String, u32)> = locked(&self.starting).drain().collect();
        let groups: Vec<u32> = starting.iter().map(|(_, group)| *group).collect();
        stop_groups(&groups, SHUTDOWN_GRACE, || true);
        for (id, _) in &starting {
            self.forget(id);
        }
    }
}

#[cfg(test)]
#[path = "claude_headless_worker_tests.rs"]
mod tests;
