use crate::cursor_owner::{now_secs, process_start, signal_group, sweep_owner, write_file};
use crate::cursor_owner::{request_ids, TERM_GRACE, TERM_POLL};
use crate::warn::warn;
use crate::worker_mcp::Inbox;
use crate::workers::headless::HeadlessHost;
use agent_client_protocol::schema::v1::{
    CancelNotification, InitializeRequest, LoadSessionRequest, McpServer, NewSessionRequest,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionId, SessionNotification,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Responder};
use futures::channel::mpsc as queue;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::future::Either;
use futures::{select, FutureExt, StreamExt};
use ralph_core::worker::{
    acp_mcp_server, auto_allow_option, cancels_turn, check_protocol, interrupt_result_text,
    is_report_tool_call, match_option, no_option_message, permission_content,
    prompt_failure_report, ready_path, request_stale, resume_decision, session_error, task_prompt,
    turn_outcome, version_warning, BlockedContent, CursorFiles, Inbound, OwnerRecord, PromptQueue,
    Resume, SessionRecord, SpawnRequest, Step, TurnOutcome, WorkerMarkers, CURSOR_ANSWER_DIR,
    CURSOR_CLOSE_DIR, CURSOR_INTERRUPT_DIR, CURSOR_INTERRUPT_RESULT_DIR, CURSOR_OWNER_DIR,
    CURSOR_PROMPT_DIR, CURSOR_SESSION_DIR, CURSOR_SPAWN_DIR, REPORT_REMINDER,
};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const AGENT_COMMAND: &str = "agent";
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

struct PendingPermission {
    responder: Responder<RequestPermissionResponse>,
    content: BlockedContent,
}

struct Permissions {
    blocked: PathBuf,
    queue: Mutex<VecDeque<PendingPermission>>,
}

impl Permissions {
    fn new(blocked: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            blocked,
            queue: Mutex::new(VecDeque::new()),
        })
    }

    fn queue(&self) -> MutexGuard<'_, VecDeque<PendingPermission>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn publish(&self, queue: &VecDeque<PendingPermission>) {
        match queue.front() {
            Some(pending) => write_file(&self.blocked, &pending.content.render()).ok(),
            None => fs::remove_file(&self.blocked).ok(),
        };
    }

    fn push(
        &self,
        request: &RequestPermissionRequest,
        responder: Responder<RequestPermissionResponse>,
    ) -> bool {
        let value = serde_json::to_value(request).unwrap_or_default();
        let content = permission_content(&value);
        if let Some(option) = auto_allow_option(&content) {
            let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            ));
            responder
                .respond(RequestPermissionResponse::new(outcome))
                .ok();
            return true;
        }
        let mut queue = self.queue();
        queue.push_back(PendingPermission { responder, content });
        self.publish(&queue);
        false
    }

    fn answer(&self, text: &str) -> Result<(), String> {
        let mut queue = self.queue();
        let front = queue
            .front()
            .ok_or("no permission request is pending for this worker")?;
        let option = match_option(&front.content.options, text)
            .ok_or_else(|| no_option_message(&front.content.options))?;
        let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        ));
        if let Some(pending) = queue.pop_front() {
            pending
                .responder
                .respond(RequestPermissionResponse::new(outcome))
                .ok();
        }
        self.publish(&queue);
        Ok(())
    }

    fn cancel_all(&self) {
        let drained: Vec<PendingPermission> = self.queue().drain(..).collect();
        for pending in drained {
            pending
                .responder
                .respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
                .ok();
        }
        fs::remove_file(&self.blocked).ok();
    }
}

pub enum Launch {
    Fresh {
        path: PathBuf,
        template: SessionRecord,
    },
    Resume {
        record: SessionRecord,
        store: PathBuf,
    },
}

pub struct Slot<'a> {
    pub id: &'a str,
    pub owner: &'a Path,
    pub results: &'a Path,
    pub markers: WorkerMarkers,
    pub starting: &'a Mutex<HashMap<String, u32>>,
}

struct Opening {
    launch: Launch,
    cwd: PathBuf,
    server: McpServer,
    loading: Arc<AtomicBool>,
    worker_id: String,
}

async fn open_session(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    loads_session: bool,
) -> Result<SessionId, String> {
    match &opening.launch {
        Launch::Fresh { .. } => connection
            .send_request(
                NewSessionRequest::new(opening.cwd.clone())
                    .mcp_servers(vec![opening.server.clone()]),
            )
            .block_task()
            .await
            .map(|created| created.session_id)
            .map_err(|e| session_error(i32::from(e.code), &e.message)),
        Launch::Resume { record, store } => {
            if let Resume::Skip(message) =
                resume_decision(&opening.worker_id, record, store, loads_session)
            {
                return Err(message);
            }
            let session = SessionId::new(record.session_id.clone());
            opening.loading.store(true, Ordering::SeqCst);
            let loaded = connection
                .send_request(
                    LoadSessionRequest::new(session.clone(), opening.cwd.clone())
                        .mcp_servers(vec![opening.server.clone()]),
                )
                .block_task()
                .await;
            opening.loading.store(false, Ordering::SeqCst);
            loaded.map(|_| session).map_err(|e| {
                format!(
                    "cursor session {} could not be loaded: {}",
                    opening.worker_id, e.message
                )
            })
        }
    }
}

pub struct AcpWorker {
    child: Child,
    permissions: Arc<Permissions>,
    stop: Option<oneshot::Sender<()>>,
    owner: PathBuf,
    prompts: queue::UnboundedSender<Inbound>,
    closing: Arc<AtomicBool>,
    done: bool,
}

impl AcpWorker {
    pub fn start(request: &SpawnRequest, launch: Launch, slot: Slot) -> Result<Self, String> {
        let Slot {
            id,
            owner,
            results,
            markers,
            starting,
        } = slot;
        if let Some(warning) = agent_version_warning() {
            warn(warning);
        }
        let mut child = Command::new(AGENT_COMMAND)
            .arg("acp")
            .current_dir(&request.worktree)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start `{AGENT_COMMAND} acp`: {e}"))?;
        let streams = child.stdin.take().zip(child.stdout.take());
        let permissions = Permissions::new(markers.blocked.clone());
        let (prompts, prompt_rx) = queue::unbounded();
        let closing = Arc::new(AtomicBool::new(false));
        let reported = Arc::new(AtomicBool::new(false));
        let mut worker = Self {
            child,
            permissions: Arc::clone(&permissions),
            stop: None,
            owner: owner.to_path_buf(),
            prompts: prompts.clone(),
            closing: Arc::clone(&closing),
            done: false,
        };
        worker.write_owner()?;
        starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_string(), worker.child.id());
        let (stdin, stdout) = streams.ok_or("cursor agent has no stdio pipes")?;
        let server = serde_json::from_value::<McpServer>(acp_mcp_server(
            &request.exe,
            id,
            &request.inbox,
            owner,
        ))
        .map_err(|e| format!("could not build the worker MCP entry: {e}"))?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        worker.stop = Some(stop_tx);
        let cwd = request.worktree.clone();
        let prompt = task_prompt(&request.task);
        let loading = Arc::new(AtomicBool::new(false));
        let replaying = Arc::clone(&loading);
        let inbox = Inbox::new(request.inbox.clone());
        let worker_id = id.to_string();
        let results = results.to_path_buf();
        let turns_permissions = Arc::clone(&permissions);
        let seen = Arc::clone(&reported);
        let allowed = Arc::clone(&reported);
        thread::spawn(move || {
            let failed = ready_tx.clone();
            let transport = ByteStreams::new(
                blocking::Unblock::new(stdin),
                blocking::Unblock::new(stdout),
            );
            let outcome = block_on(
                Client
                    .builder()
                    .on_receive_request(
                        async move |request: RequestPermissionRequest, responder, _cx| {
                            if permissions.push(&request, responder) {
                                allowed.store(true, Ordering::SeqCst);
                            }
                            Ok(())
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_notification(
                        async move |notification: SessionNotification, _cx| {
                            let update =
                                serde_json::to_value(&notification.update).unwrap_or_default();
                            if !replaying.load(Ordering::SeqCst) && is_report_tool_call(&update) {
                                seen.store(true, Ordering::SeqCst);
                            }
                            Ok(())
                        },
                        agent_client_protocol::on_receive_notification!(),
                    )
                    .connect_with(transport, |connection: ConnectionTo<Agent>| async move {
                        let init = connection
                            .send_request(InitializeRequest::new(ProtocolVersion::V1))
                            .block_task()
                            .await?;
                        if let Err(message) = check_protocol(init.protocol_version.as_u16()) {
                            ready_tx.send(Err(message)).ok();
                            return Ok(());
                        }
                        let opening = Opening {
                            launch,
                            cwd,
                            server,
                            loading,
                            worker_id: worker_id.clone(),
                        };
                        let loads = init.agent_capabilities.load_session;
                        match open_session(&connection, &opening, loads).await {
                            Ok(session) => {
                                if let Launch::Fresh { path, template } = &opening.launch {
                                    let record = SessionRecord {
                                        session_id: session.to_string(),
                                        ..template.clone()
                                    };
                                    if let Err(e) = write_file(path, &record.render()) {
                                        warn(format!(
                                            "could not write the cursor session record: {e}"
                                        ));
                                    }
                                    prompts.unbounded_send(Inbound::Prompt(prompt)).ok();
                                }
                                ready_tx.send(Ok(())).ok();
                                let turns = Turns {
                                    permissions: Arc::clone(&turns_permissions),
                                    session,
                                    worker_id,
                                    markers,
                                    results,
                                    inbox,
                                    reported,
                                    closing,
                                };
                                run_turns(&connection, &turns, prompt_rx, stop_rx).await;
                            }
                            Err(message) => {
                                ready_tx.send(Err(message)).ok();
                            }
                        }
                        Ok(())
                    }),
            );
            if let Err(e) = outcome {
                failed
                    .send(Err(format!("cursor agent connection failed: {e}")))
                    .ok();
            }
        });
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(())) => Ok(worker),
            Ok(Err(message)) => Err(message),
            Err(_) => Err(format!(
                "cursor agent was not ready after {}s",
                READY_TIMEOUT.as_secs()
            )),
        }
    }

    fn nudge(&self, text: &str) {
        self.prompts
            .unbounded_send(Inbound::Prompt(text.to_string()))
            .ok();
    }

    fn interrupt(&self, text: &str, entry: &str) {
        self.prompts
            .unbounded_send(Inbound::Interrupt(text.to_string(), entry.to_string()))
            .ok();
    }

    fn write_owner(&self) -> Result<(), String> {
        let chat = process::id();
        let group = self.child.id();
        let record = OwnerRecord {
            chat_pid: chat,
            chat_start: process_start(chat).ok_or("could not read the chat start time")?,
            group,
            group_start: process_start(group)
                .ok_or("could not read the cursor agent start time")?,
        };
        write_file(&self.owner, &record.render())
            .map_err(|e| format!("could not write the cursor owner file: {e}"))
    }

    pub fn terminate(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        self.closing.store(true, Ordering::SeqCst);
        self.permissions.cancel_all();
        if let Some(stop) = self.stop.take() {
            stop.send(()).ok();
        }
        let group = self.child.id();
        signal_group(group, "TERM");
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            thread::sleep(TERM_POLL);
        }
        signal_group(group, "KILL");
        self.child.wait().ok();
        fs::remove_file(&self.owner).ok();
    }
}

impl Drop for AcpWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct Turns {
    permissions: Arc<Permissions>,
    session: SessionId,
    worker_id: String,
    markers: WorkerMarkers,
    results: PathBuf,
    inbox: Inbox,
    reported: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
}

fn run_steps(connection: &ConnectionTo<Agent>, turns: &Turns, steps: Vec<Step>) {
    for step in steps {
        match step {
            Step::CancelPermissions => turns.permissions.cancel_all(),
            Step::CancelSession => {
                connection
                    .send_notification(CancelNotification::new(turns.session.clone()))
                    .ok();
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

async fn run_turns(
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
        turns.reported.store(false, Ordering::SeqCst);
        let mut sent = Box::pin(
            connection
                .send_request(PromptRequest::new(
                    turns.session.clone(),
                    vec![text.clone().into()],
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
        let interrupted = queue.finish();
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
            TurnOutcome::Problem(problem) => {
                let at = chrono::Utc::now().to_rfc3339();
                let report = prompt_failure_report(&turns.worker_id, &at, &problem);
                turns.inbox.post(&report, 0).ok();
            }
            TurnOutcome::Cancelled => turns.permissions.cancel_all(),
            TurnOutcome::Reported | TurnOutcome::Closed => {}
        }
    }
}

fn agent_version_warning() -> Option<String> {
    let output = Command::new(AGENT_COMMAND)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    version_warning(&String::from_utf8_lossy(&output.stdout))
}

pub struct CursorHost {
    store_dir: PathBuf,
    workers: Mutex<HashMap<String, AcpWorker>>,
    starting: Mutex<HashMap<String, u32>>,
    closed: AtomicBool,
}

impl CursorHost {
    pub fn new(store_dir: PathBuf) -> Arc<Self> {
        for id in request_ids(&store_dir, CURSOR_OWNER_DIR) {
            sweep_owner(&store_dir, &id);
        }
        fs::remove_dir_all(store_dir.join(CURSOR_INTERRUPT_RESULT_DIR)).ok();
        Arc::new(Self {
            store_dir,
            workers: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    fn workers(&self) -> MutexGuard<'_, HashMap<String, AcpWorker>> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn files(&self, id: &str) -> CursorFiles {
        CursorFiles::new(&self.store_dir, id)
    }

    fn poll(&self) {
        for id in request_ids(&self.store_dir, CURSOR_CLOSE_DIR) {
            self.close(&id);
        }
        for id in request_ids(&self.store_dir, CURSOR_ANSWER_DIR) {
            self.answer(&id);
        }
        for id in request_ids(&self.store_dir, CURSOR_PROMPT_DIR) {
            self.prompt(
                &id,
                |files| &files.prompt,
                |worker, text, _| worker.nudge(text),
            );
        }
        for id in request_ids(&self.store_dir, CURSOR_INTERRUPT_DIR) {
            self.prompt(&id, |files| &files.interrupt, AcpWorker::interrupt);
        }
        for id in request_ids(&self.store_dir, CURSOR_SPAWN_DIR) {
            self.spawn(&id);
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let drained: Vec<(String, AcpWorker)> = self.workers().drain().collect();
        for (id, mut worker) in drained {
            worker.terminate();
            self.forget(&id);
        }
        let starting: Vec<(String, u32)> = self
            .starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .collect();
        for (_, group) in &starting {
            signal_group(*group, "TERM");
        }
        if !starting.is_empty() {
            thread::sleep(SHUTDOWN_GRACE);
        }
        for (id, group) in &starting {
            signal_group(*group, "KILL");
            self.forget(id);
        }
    }

    fn forget(&self, id: &str) {
        let files = self.files(id);
        fs::remove_file(&files.owner).ok();
        fs::remove_file(&files.answer).ok();
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_dir_all(&files.interrupt).ok();
        fs::remove_dir_all(&files.interrupt_result).ok();
        fs::remove_file(ready_path(&self.store_dir, id)).ok();
    }

    pub fn cancel_pending(&self, id: &str) {
        let permissions = self
            .workers()
            .get(id)
            .map(|worker| Arc::clone(&worker.permissions));
        if let Some(permissions) = permissions {
            permissions.cancel_all();
        }
    }

    fn answer(&self, id: &str) {
        let files = self.files(id);
        let Ok(text) = fs::read_to_string(&files.answer) else {
            return;
        };
        let permissions = self
            .workers()
            .get(id)
            .map(|worker| Arc::clone(&worker.permissions));
        let result = permissions
            .ok_or_else(|| "worker has no running cursor agent".to_string())
            .and_then(|permissions| permissions.answer(&text));
        if let Err(message) = result {
            write_file(&files.error, &message).ok();
        }
        fs::remove_file(&files.answer).ok();
    }

    fn prompt(
        &self,
        id: &str,
        dir: fn(&CursorFiles) -> &PathBuf,
        deliver: fn(&AcpWorker, &str, &str),
    ) {
        let files = self.files(id);
        let starting = self
            .starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(id);
        if starting && !self.workers().contains_key(id) {
            return;
        }
        let mut entries: Vec<PathBuf> = fs::read_dir(dir(&files))
            .map(|dir| {
                dir.filter_map(Result::ok)
                    .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default();
        entries.sort();
        for entry in entries {
            let name = entry
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let (Ok(text), Some(worker)) = (fs::read_to_string(&entry), self.workers().get(id)) {
                deliver(worker, &text, &name);
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn close(&self, id: &str) {
        self.cancel_pending(id);
        let removed = self.workers().remove(id);
        if let Some(mut worker) = removed {
            worker.terminate();
        }
        self.forget(id);
        fs::remove_file(&self.files(id).session).ok();
        fs::remove_file(&self.files(id).close).ok();
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
        if self.workers().contains_key(id) {
            self.fail(id, "worker is already running");
            return;
        }
        let launch = Launch::Fresh {
            path: files.session.clone(),
            template: SessionRecord {
                session_id: String::new(),
                cwd: request.worktree.clone(),
                inbox: request.inbox.clone(),
                store: Some(self.store_dir.clone()),
            },
        };
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
    ) -> Result<AcpWorker, String> {
        let started = AcpWorker::start(
            request,
            launch,
            Slot {
                id,
                owner: &self.files(id).owner,
                results: &self.files(id).interrupt_result,
                markers: WorkerMarkers::new(&self.store_dir, id),
                starting: &self.starting,
            },
        );
        self.starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        started
    }

    fn resume_all(&self) {
        for id in request_ids(&self.store_dir, CURSOR_SESSION_DIR) {
            self.resume(&id);
        }
    }

    fn resume(&self, id: &str) {
        let files = self.files(id);
        if files.owner.exists() {
            warn(format!(
                "cursor session {id} not resumed: a running chat owns it"
            ));
            return;
        }
        let Some(record) = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| SessionRecord::parse(&text))
        else {
            warn(format!(
                "cursor session {id} not resumed: its record is unreadable"
            ));
            return;
        };
        if let Resume::Skip(message) = resume_decision(id, &record, &self.store_dir, true) {
            warn(message);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            warn(format!(
                "cursor session {id} not resumed: could not find the ralph executable"
            ));
            return;
        };
        if !record.cwd.is_dir() {
            warn(format!(
                "cursor session {id} not resumed: its worktree {} is gone",
                record.cwd.display()
            ));
            return;
        }
        let request = SpawnRequest {
            worktree: record.cwd.clone(),
            exe,
            inbox: record.inbox.clone(),
            task: String::new(),
            at: now_secs(),
        };
        let launch = Launch::Resume {
            record,
            store: self.store_dir.clone(),
        };
        match self.launch(id, &request, launch) {
            Ok(worker) => self.adopt_resumed(id, worker),
            Err(message) => {
                warn(message.clone());
                write_file(&files.error, &message).ok();
            }
        }
    }

    fn adopt_resumed(&self, id: &str, worker: AcpWorker) {
        let ready = ready_path(&self.store_dir, id);
        if !self.closed.load(Ordering::SeqCst) && write_file(&ready, "").is_ok() {
            self.workers().insert(id.to_string(), worker);
        } else {
            drop(worker);
            fs::remove_file(ready).ok();
        }
    }

    fn adopt(&self, id: &str, worker: AcpWorker) {
        let files = self.files(id);
        let claimed = !self.closed.load(Ordering::SeqCst)
            && fs::remove_file(&files.spawn).is_ok()
            && write_file(&ready_path(&self.store_dir, id), "").is_ok();
        if claimed {
            self.workers().insert(id.to_string(), worker);
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

impl HeadlessHost for CursorHost {
    fn resume(&self) {
        self.resume_all();
    }

    fn poll(&self) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        CursorHost::poll(self);
        true
    }

    fn shutdown(&self) {
        CursorHost::shutdown(self);
    }
}
