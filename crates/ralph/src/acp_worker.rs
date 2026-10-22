use crate::acp_log::{CappedLog, LoggedRead, LoggedWrite};
use crate::config::AppConfig;
use crate::cursor_owner::request_ids;
use crate::cursor_owner::{now_secs, process_start, sweep_owner, write_file};
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::warn::warn;
use crate::worker_changes::WorkerChanges;
use crate::worker_mcp::Inbox;
use crate::workers::headless::{HeadlessHost, Heartbeat};
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
use ralph_core::snapshot::{snapshots_enabled, WorkerSite};
use ralph_core::worker::{
    acp_mcp_server, activity_path, auto_allow_option, cancels_turn, capabilities, check_protocol,
    decide, decode_update, describe, encode, gone_path, interrupt_result_text, match_option,
    no_option_message, permission_alert_report, permission_content, prompt_failure_report,
    ready_path, reduce, request_stale, resume_decision, session_error, session_saved, stop_problem,
    task_prompt, turn_outcome, version_warning, AcpAgent, AcpCall, BlockedContent, Capabilities,
    CursorPermissions, HandoffFiles, HarnessId, HeadlessEffect, HeadlessState, HeadlessWorker,
    Inbound, Marker, OwnerRecord, PromptQueue, Resume, Scope, SessionRecord, SpawnRequest, Step,
    TurnEnd, TurnOutcome, WorkerCommand, WorkerEvent, WorkerMarkers, CURSOR_AGENT,
    CURSOR_ANSWER_DIR, CURSOR_CLOSE_DIR, CURSOR_INTERRUPT_DIR, CURSOR_INTERRUPT_RESULT_DIR,
    CURSOR_OWNER_DIR, CURSOR_PROMPT_DIR, CURSOR_SESSION_DIR, CURSOR_SPAWN_DIR, REPORT_REMINDER,
};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

mod model;
mod permissions;
mod turns;

use permissions::Permissions;
use turns::{run_turns, Turns};

pub(crate) type Events = Arc<Mutex<Vec<WorkerEvent>>>;

pub(crate) fn push_event(events: &Events, event: WorkerEvent) {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(event);
}

pub(crate) fn apply_effect(markers: &WorkerMarkers, effect: &HeadlessEffect) {
    let path = |marker| match marker {
        Marker::Ready => &markers.ready,
        Marker::Reported => &markers.reported,
        Marker::Blocked => &markers.blocked,
        Marker::Waiting => &markers.waiting,
    };
    match effect {
        HeadlessEffect::Set(marker) => {
            write_file(path(*marker), "").ok();
        }
        HeadlessEffect::Clear(marker) => {
            fs::remove_file(path(*marker)).ok();
        }
        HeadlessEffect::Publish(content) => {
            write_file(&markers.blocked, &content.render()).ok();
        }
        HeadlessEffect::ReplyText(_) => {}
    }
}

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

fn cursor_permissions() -> CursorPermissions {
    match AppConfig::load() {
        Ok(config) => {
            let (mode, warning) = config.cursor.permissions_setting();
            if let Some(warning) = warning {
                warn(warning);
            }
            mode
        }
        Err(e) => {
            warn(format!(
                "could not read config: {e}; cursor permissions use \"ask\""
            ));
            CursorPermissions::Ask
        }
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
    pub agent: AcpAgent,
    pub id: &'a str,
    pub owner: &'a Path,
    pub results: &'a Path,
    pub markers: WorkerMarkers,
    pub starting: &'a Mutex<HashMap<String, u32>>,
}

struct Opening {
    agent: AcpAgent,
    launch: Launch,
    cwd: PathBuf,
    server: McpServer,
    loading: Arc<AtomicBool>,
    worker_id: String,
    model: Option<String>,
}

async fn open_session(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    loads_session: bool,
) -> Result<SessionId, String> {
    match &opening.launch {
        Launch::Fresh { .. } => {
            let created = connection
                .send_request(
                    NewSessionRequest::new(opening.cwd.clone())
                        .mcp_servers(vec![opening.server.clone()]),
                )
                .block_task()
                .await
                .map_err(|e| session_error(&opening.agent, i32::from(e.code), &e.message))?;
            model::apply_model(
                connection,
                opening,
                &created.session_id,
                created.config_options.as_deref(),
            )
            .await?;
            Ok(created.session_id)
        }
        Launch::Resume { record, store } => {
            if let Resume::Skip(message) = resume_decision(
                &opening.agent,
                &opening.worker_id,
                record,
                store,
                loads_session,
            ) {
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
            let loaded = loaded.map_err(|e| {
                format!(
                    "{} session {} could not be loaded: {}",
                    opening.agent.name, opening.worker_id, e.message
                )
            })?;
            model::apply_model(
                connection,
                opening,
                &session,
                loaded.config_options.as_deref(),
            )
            .await?;
            Ok(session)
        }
    }
}

pub struct AcpWorker {
    child: Child,
    permissions: Arc<Permissions>,
    stop: Option<oneshot::Sender<()>>,
    finished: mpsc::Receiver<()>,
    owner: PathBuf,
    prompts: queue::UnboundedSender<Inbound>,
    closing: Arc<AtomicBool>,
    done: bool,
    exit_seen: bool,
    caps: Capabilities,
    markers: WorkerMarkers,
    state: HeadlessState,
    events: Events,
}

impl AcpWorker {
    pub fn start(request: &SpawnRequest, launch: Launch, slot: Slot) -> Result<Self, String> {
        let Slot {
            agent,
            id,
            owner,
            results,
            markers,
            starting,
        } = slot;
        if let Some(warning) = agent_version_warning(&agent) {
            warn(warning);
        }
        let mut command = Command::new(agent.command);
        command.arg("acp").current_dir(&request.worktree);
        let (child, stdin, stdout, stderr) = spawn_piped(command)
            .map_err(|e| format!("could not start `{} acp`: {e}", agent.command))?;
        let log = CappedLog::new(request.inbox.parent(), id);
        let stderr_log = Arc::clone(&log);
        thread::spawn(move || {
            let mut stderr = LoggedRead {
                inner: stderr,
                log: stderr_log,
                tag: "err",
            };
            io::copy(&mut stderr, &mut io::sink()).ok();
        });
        let events = Events::default();
        let permissions = Permissions::new(
            &markers,
            cursor_permissions(),
            id.to_string(),
            Inbox::new(request.inbox.clone()),
            Arc::clone(&events),
        );
        let (prompts, prompt_rx) = queue::unbounded();
        let (finished_tx, finished_rx) = mpsc::channel();
        let closing = Arc::new(AtomicBool::new(false));
        let reported = Arc::new(AtomicBool::new(false));
        let mut worker = Self {
            child,
            permissions: Arc::clone(&permissions),
            stop: None,
            finished: finished_rx,
            owner: owner.to_path_buf(),
            prompts: prompts.clone(),
            closing: Arc::clone(&closing),
            done: false,
            exit_seen: false,
            caps: capabilities(HarnessId::Cursor),
            markers: markers.clone(),
            state: HeadlessState::default(),
            events: Arc::clone(&events),
        };
        worker.write_owner(&agent)?;
        starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_string(), worker.child.id());
        let server = serde_json::from_value::<McpServer>(acp_mcp_server(
            &request.exe,
            id,
            &request.inbox,
            owner,
            request.snapshots.then_some(request.worktree.as_path()),
        ))
        .map_err(|e| format!("could not build the worker MCP entry: {e}"))?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        worker.stop = Some(stop_tx);
        let cwd = request.worktree.clone();
        let prompt = task_prompt(&request.task);
        let loading = Arc::new(AtomicBool::new(false));
        let replaying = Arc::clone(&loading);
        let updated = Arc::new(AtomicBool::new(false));
        let touched = Arc::clone(&updated);
        let inbox = Inbox::new(request.inbox.clone());
        let worker_id = id.to_string();
        let changes = request
            .inbox
            .parent()
            .filter(|_| request.snapshots)
            .and_then(|store| {
                WorkerChanges::open(&WorkerSite {
                    store: store.to_path_buf(),
                    id: id.to_string(),
                    worktree: request.worktree.clone(),
                })
            });
        let results = results.to_path_buf();
        let turns_permissions = Arc::clone(&permissions);
        let seen = Arc::clone(&reported);
        let allowed = Arc::clone(&reported);
        let heartbeat = Arc::clone(&permissions.heartbeat);
        heartbeat.clear();
        let on_request = Arc::clone(&heartbeat);
        let on_notification = Arc::clone(&heartbeat);
        let in_turns = Arc::clone(&heartbeat);
        let ended = Arc::clone(&closing);
        let opened = Arc::new(AtomicBool::new(false));
        let was_opened = Arc::clone(&opened);
        let session_model = request.model.clone();
        thread::spawn(move || {
            let failed = ready_tx.clone();
            let transport = ByteStreams::new(
                blocking::Unblock::new(LoggedWrite {
                    inner: stdin,
                    log: Arc::clone(&log),
                    tag: "in",
                }),
                blocking::Unblock::new(LoggedRead {
                    inner: stdout,
                    log,
                    tag: "out",
                }),
            );
            let outcome = block_on(
                Client
                    .builder()
                    .on_receive_request(
                        async move |request: RequestPermissionRequest, responder, _cx| {
                            on_request.touch();
                            if permissions.push(&request, responder) {
                                allowed.store(true, Ordering::SeqCst);
                            }
                            Ok(())
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_notification(
                        async move |notification: SessionNotification, _cx| {
                            if !replaying.load(Ordering::SeqCst) {
                                touched.store(true, Ordering::SeqCst);
                            }
                            let update =
                                serde_json::to_value(&notification.update).unwrap_or_default();
                            for event in decode_update(&update, now_secs()) {
                                match event {
                                    WorkerEvent::Activity { at } => on_notification.touch_at(at),
                                    WorkerEvent::Reported if !replaying.load(Ordering::SeqCst) => {
                                        seen.store(true, Ordering::SeqCst);
                                    }
                                    _ => {}
                                }
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
                        if let Err(message) = check_protocol(&agent, init.protocol_version.as_u16())
                        {
                            ready_tx.send(Err(message)).ok();
                            return Ok(());
                        }
                        let opening = Opening {
                            agent,
                            launch,
                            cwd,
                            server,
                            loading,
                            worker_id: worker_id.clone(),
                            model: session_model,
                        };
                        let loads = init.agent_capabilities.load_session;
                        match open_session(&connection, &opening, loads).await {
                            Ok(session) => {
                                let pending = match &opening.launch {
                                    Launch::Fresh { path, template } => {
                                        prompts.unbounded_send(Inbound::Prompt(prompt)).ok();
                                        Some((
                                            path.clone(),
                                            SessionRecord {
                                                session_id: session.to_string(),
                                                ..template.clone()
                                            },
                                        ))
                                    }
                                    Launch::Resume { .. } => None,
                                };
                                opened.store(true, Ordering::SeqCst);
                                ready_tx.send(Ok(())).ok();
                                let turns = Turns {
                                    permissions: Arc::clone(&turns_permissions),
                                    agent,
                                    session,
                                    worker_id,
                                    markers,
                                    results,
                                    inbox,
                                    reported,
                                    closing,
                                    changes,
                                    heartbeat: in_turns,
                                    events,
                                    updated,
                                    pending,
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
            if was_opened.load(Ordering::SeqCst) && !ended.load(Ordering::SeqCst) {
                let reason = match &outcome {
                    Ok(()) => format!("{} acp exited", agent.command),
                    Err(e) => e.to_string(),
                };
                heartbeat.gone(&Scope::Worker, &reason);
            }
            if let Err(e) = outcome {
                failed
                    .send(Err(format!("{} agent connection failed: {e}", agent.name)))
                    .ok();
            }
            finished_tx.send(()).ok();
        });
        match ready_rx.recv_timeout(READY_TIMEOUT) {
            Ok(Ok(())) => Ok(worker),
            Ok(Err(message)) => Err(message),
            Err(_) => Err(format!(
                "{} agent was not ready after {}s",
                agent.name,
                READY_TIMEOUT.as_secs()
            )),
        }
    }

    fn interrupt(&self, text: &str, entry: &str) {
        self.prompts
            .unbounded_send(Inbound::Interrupt(text.to_string(), entry.to_string()))
            .ok();
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

    fn write_owner(&self, agent: &AcpAgent) -> Result<(), String> {
        let chat = process::id();
        let group = self.child.id();
        let record = OwnerRecord {
            chat_pid: chat,
            chat_start: process_start(chat).ok_or("could not read the chat start time")?,
            group,
            group_start: process_start(group)
                .ok_or_else(|| format!("could not read the {} agent start time", agent.name))?,
        };
        write_file(&self.owner, &record.render())
            .map_err(|e| format!("could not write the {} owner file: {e}", agent.name))
    }

    fn reap_exit(&mut self) {
        if self.done || self.exit_seen {
            return;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            self.exit_seen = true;
            if !self.closing.load(Ordering::SeqCst) {
                self.permissions
                    .heartbeat
                    .gone(&Scope::Worker, &exit_reason(status));
            }
        }
    }

    fn begin_stop(&mut self) {
        self.done = true;
        self.closing.store(true, Ordering::SeqCst);
        self.permissions.cancel_all();
        if let Some(stop) = self.stop.take() {
            stop.send(()).ok();
        }
    }

    fn finish_stop(&mut self, deadline: Instant) {
        self.finished
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok();
        stop_child(&mut self.child);
        fs::remove_file(&self.owner).ok();
    }

    pub fn terminate(&mut self) {
        if self.done {
            return;
        }
        self.begin_stop();
        self.finish_stop(Instant::now() + SHUTDOWN_GRACE);
    }
}

impl HeadlessWorker for AcpWorker {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn send(&mut self, command: WorkerCommand) -> Result<(), String> {
        let command = decide(&self.caps, command).map_err(|event| match event {
            WorkerEvent::Unsupported { reason, .. } => reason,
            other => format!("{other:?}"),
        })?;
        match command {
            WorkerCommand::Prompt(text) => {
                self.prompts.unbounded_send(Inbound::Prompt(text)).ok();
                Ok(())
            }
            WorkerCommand::AnswerPermission { option_id } => self.permissions.answer(&option_id),
            WorkerCommand::Close => {
                self.terminate();
                push_event(&self.events, WorkerEvent::Closed);
                Ok(())
            }
            WorkerCommand::Spawn(_) | WorkerCommand::Resume | WorkerCommand::Interrupt(_) => {
                Err(format!(
                    "not sent: the host handles it ({})",
                    describe(&self.caps, command.feature())
                ))
            }
        }
    }

    fn poll(&mut self) -> Vec<WorkerEvent> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Drop for AcpWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn exit_reason(status: ExitStatus) -> String {
    format!("{} acp exited ({status})", CURSOR_AGENT.command)
}

fn agent_version_warning(agent: &AcpAgent) -> Option<String> {
    let output = Command::new(agent.command)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    version_warning(agent, &String::from_utf8_lossy(&output.stdout))
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

    fn files(&self, id: &str) -> HandoffFiles {
        HandoffFiles::cursor(&self.store_dir, id)
    }

    fn reap_exits(&self) {
        for worker in self.workers().values_mut() {
            worker.reap_exit();
            worker.apply_events();
        }
    }

    fn poll(&self) {
        self.reap_exits();
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
                |worker, text, _| {
                    if let Err(message) = worker.send(WorkerCommand::Prompt(text.to_string())) {
                        warn(message);
                    }
                },
            );
        }
        for id in request_ids(&self.store_dir, CURSOR_INTERRUPT_DIR) {
            self.prompt(
                &id,
                |files| &files.interrupt,
                |worker, text, entry| worker.interrupt(text, entry),
            );
        }
        for id in request_ids(&self.store_dir, CURSOR_SPAWN_DIR) {
            self.spawn(&id);
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut drained: Vec<(String, AcpWorker)> = self.workers().drain().collect();
        for (_, worker) in &mut drained {
            worker.begin_stop();
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        for (id, worker) in &mut drained {
            worker.finish_stop(deadline);
            self.forget(id);
        }
        let starting: Vec<(String, u32)> = self
            .starting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain()
            .collect();
        if starting.is_empty() {
            return;
        }
        let groups: Vec<u32> = starting.iter().map(|(_, group)| *group).collect();
        stop_groups(&groups, SHUTDOWN_GRACE, || true);
        for (id, _) in &starting {
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
        fs::remove_file(activity_path(&self.store_dir, id)).ok();
        fs::remove_file(gone_path(&self.store_dir, id)).ok();
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
        let result = self
            .workers()
            .get_mut(id)
            .ok_or_else(|| format!("worker has no running {} agent", CURSOR_AGENT.name))
            .and_then(|worker| worker.send(WorkerCommand::AnswerPermission { option_id: text }));
        if let Err(message) = result {
            write_file(&files.error, &message).ok();
        }
        fs::remove_file(&files.answer).ok();
    }

    fn prompt(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&mut AcpWorker, &str, &str),
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
            if let (Ok(text), Some(worker)) =
                (fs::read_to_string(&entry), self.workers().get_mut(id))
            {
                deliver(worker, &text, &name);
            }
            fs::remove_file(&entry).ok();
        }
    }

    fn close(&self, id: &str) {
        self.cancel_pending(id);
        let removed = self.workers().remove(id);
        if let Some(mut worker) = removed {
            worker.send(WorkerCommand::Close).ok();
            worker.apply_events();
        }
        self.forget(id);
        fs::remove_file(ready_path(&self.store_dir, id)).ok();
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
                model: request.model.clone(),
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
                agent: CURSOR_AGENT,
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
                "{} session {id} not resumed: a running chat owns it",
                CURSOR_AGENT.name
            ));
            return;
        }
        let Some(record) = fs::read_to_string(&files.session)
            .ok()
            .and_then(|text| SessionRecord::parse(&text))
        else {
            warn(format!(
                "{} session {id} not resumed: its record is unreadable",
                CURSOR_AGENT.name
            ));
            return;
        };
        if let Resume::Skip(message) =
            resume_decision(&CURSOR_AGENT, id, &record, &self.store_dir, true)
        {
            warn(message);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            warn(format!(
                "{} session {id} not resumed: could not find the ralph executable",
                CURSOR_AGENT.name
            ));
            return;
        };
        if !record.cwd.is_dir() {
            warn(format!(
                "{} session {id} not resumed: its worktree {} is gone",
                CURSOR_AGENT.name,
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
            snapshots: snapshots_enabled(
                AppConfig::load().ok().map(|config| config.changes.enabled),
            ),
            model: record.model.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn exit_reason_names_the_agent_and_the_status() {
        assert_eq!(
            exit_reason(ExitStatus::from_raw(9)),
            "agent acp exited (signal: 9 (SIGKILL))"
        );
    }
}
