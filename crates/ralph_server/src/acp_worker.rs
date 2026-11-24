use crate::acp_log::{drain_stderr, CappedLog};
use crate::config::{configured_permission_mode, AppConfig};
use crate::cursor_owner::request_ids;
use crate::cursor_owner::{
    claim_owner, now_secs, report_start_warning, sweep_owner, take_answer, write_file,
    write_interrupt_result, write_owner_record,
};
use crate::subprocess::{exited_unreaped, signal_group, unregister_group};
use crate::transport::stdio::{spawn_piped, stop_child, stop_groups};
use crate::warn::warn;
use crate::watches::locked;
use crate::worker_changes::WorkerChanges;
use crate::worker_events::{apply_effect, push_event, Events};
use crate::worker_mcp::Inbox;
use crate::worker_shell::{in_shell, worker_shell};
use crate::workers::headless::{mark_prompt_lost, report_interrupt_gone, HeadlessHost, Heartbeat};
use agent_client_protocol::schema::v1::{
    AuthenticateRequest, CancelNotification, CloseSessionRequest, InitializeResponse,
    LoadSessionRequest, McpServer, NewSessionRequest, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, SessionId,
    SessionNotification,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Responder};
use futures::channel::mpsc as queue;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::future::Either;
use futures::{select, FutureExt, StreamExt};
use ralph_core::snapshot::{snapshots_enabled, WorkerSite};
use ralph_core::worker::{
    acp_mcp_server, activity_path, agy_auto_allow_option, auth_method, auto_allow_option,
    cancels_turn, capabilities, decide, describe, encode, gone_path, permission_content,
    probed_version, prompt_failure_report, ready_path, reduce, request_stale, session_error,
    stop_problem, sweeps_permissions, task_prompt, turn_outcome, AcpAgent, AcpCall, BlockedContent,
    Capabilities, HandoffFiles, HarnessId, HeadlessState, HeadlessWorker, Inbound,
    PermissionAnswer, PermissionMode, PromptQueue, Resume, Scope, SessionRecord, SpawnRequest,
    Step, TurnEnd, TurnOutcome, WorkerCommand, WorkerEvent, WorkerMarkers, AGY_AGENT, AGY_GATE,
    CURSOR_AGENT, CURSOR_ANSWER_DIR, CURSOR_CLOSE_DIR, CURSOR_GATE, CURSOR_INTERRUPT_DIR,
    CURSOR_INTERRUPT_RESULT_DIR, CURSOR_OWNER_DIR, CURSOR_PROMPT_DIR, CURSOR_SESSION_DIR,
    CURSOR_SPAWN_DIR, REPORT_REMINDER,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

mod agy_settings;
pub(crate) mod agy_usage;
mod agy_verify;
mod cancel;
mod cursor_worker;
pub(crate) mod journal;
mod model;
mod permissions;
mod print;
mod print_hooks;
mod queued;
pub(crate) mod recovery;
pub(crate) mod transport;
mod turns;
mod verify;

use crate::permission_queue::{host_process_token, PermissionQueue};
pub(crate) use agy_verify::{verify as verify_agy, LiveRun};
use cursor_worker::CursorWorker;
use permissions::{Permissions, RawPermissionRequest};
use print::PrintWorker;
use recovery::{agent_version_warning, exit_reason};
use transport::initialize;
use turns::{run_turns, Turns};
use verify::agent_version_probe;
pub(crate) use verify::verify;

const READY_TIMEOUT: Duration = Duration::from_secs(20);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

fn cursor_permissions() -> PermissionMode {
    configured_permission_mode("cursor", |config| config.cursor.permissions_setting())
}

pub enum Launch {
    Fresh,
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
    worker_id: String,
    model: Option<String>,
    permission_mode: PermissionMode,
    effort: Option<String>,
    updates: agy_settings::Updates,
    inbox: PathBuf,
}

async fn open_session(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    loads_session: bool,
) -> Result<(SessionId, Option<agy_settings::Selection>), String> {
    match &opening.launch {
        Launch::Fresh => agy_settings::open(connection, opening)
            .await
            .map(|(id, settings)| (id, Some(settings))),
        Launch::Resume { record, store } => {
            if let Resume::Skip(message) = recovery::resume(
                &opening.agent,
                &opening.worker_id,
                record,
                store,
                loads_session,
            ) {
                return Err(message);
            }
            let session = SessionId::new(record.session_id.clone());
            if opening.agent == AGY_AGENT {
                return agy_settings::load(connection, opening, session)
                    .await
                    .map(|(id, settings)| (id, Some(settings)));
            }
            let loaded = connection
                .send_request(
                    LoadSessionRequest::new(session.clone(), opening.cwd.clone())
                        .mcp_servers(vec![opening.server.clone()]),
                )
                .block_task()
                .await;
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
            Ok((session, None))
        }
    }
}

pub struct AcpWorker {
    agent: AcpAgent,
    child: Child,
    permissions: Arc<Permissions>,
    stop: Option<oneshot::Sender<()>>,
    finished: mpsc::Receiver<()>,
    owner: PathBuf,
    prompts: queue::UnboundedSender<Inbound>,
    journal: Option<PathBuf>,
    generation: Arc<AtomicU64>,
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
        if agent == CURSOR_AGENT {
            if let Some(warning) = agent_version_warning(&agent, &request.inbox) {
                report_start_warning(warning, None);
            }
        }
        if agent == AGY_AGENT {
            let version = agent_version_probe(&agent);
            let version = probed_version(&agent, &version)?;
            if let Some(warning) = crate::version_drift::gate(&AGY_GATE, version, &request.inbox)? {
                report_start_warning(
                    warning,
                    Some(
                        &HandoffFiles::agy(
                            request.inbox.parent().ok_or("worker inbox has no store")?,
                            id,
                        )
                        .warning,
                    ),
                );
            }
        }
        let permission_mode = if agent == AGY_AGENT {
            match &launch {
                Launch::Resume { record, .. } => record.permissions.unwrap_or(PermissionMode::Ask),
                Launch::Fresh => {
                    configured_permission_mode("agy", |config| config.agy.permissions_setting())
                }
            }
        } else {
            cursor_permissions()
        };
        let effort = if agent == AGY_AGENT {
            match &launch {
                Launch::Resume { record, .. } => record.effort.clone(),
                Launch::Fresh => {
                    let config = AppConfig::load().map_err(|error| error.to_string())?.agy;
                    if let Some(error) = config.selection_errors().into_iter().next() {
                        return Err(error);
                    }
                    config.effort
                }
            }
        } else {
            None
        };
        let cwd = if agent == AGY_AGENT {
            request
                .worktree
                .canonicalize()
                .map_err(|e| format!("could not resolve agy worktree: {e}"))?
        } else {
            request.worktree.clone()
        };
        let mut command = agy_settings::command(agent.command, permission_mode);
        command.current_dir(&cwd);
        if agent == CURSOR_AGENT {
            let store = request.inbox.parent().ok_or("worker inbox has no store")?;
            command.envs([ralph_core::worker::inbox_env(
                &fs::canonicalize(store).map_err(|e| e.to_string())?,
            )]);
        }
        let journal = if agent == AGY_AGENT {
            Some(journal::directory(
                request.inbox.parent().ok_or("worker inbox has no store")?,
                id,
            ))
        } else {
            None
        };
        let (child, stdin, stdout, stderr) = spawn_piped(in_shell(&worker_shell(), command))
            .map_err(|e| format!("could not start `{} acp`: {e}", agent.command))?;
        let log = CappedLog::new(request.inbox.parent(), id);
        drain_stderr(stderr, Arc::clone(&log));
        let events = Events::default();
        let permissions = Permissions::new(
            &markers,
            permission_mode,
            agent == AGY_AGENT,
            PermissionQueue::new(
                host_process_token(),
                id.to_string(),
                Inbox::new(request.inbox.clone()),
                Arc::clone(&events),
            ),
        );
        let (prompts, prompt_rx) = queue::unbounded();
        let (finished_tx, finished_rx) = mpsc::channel();
        let closing = Arc::new(AtomicBool::new(false));
        let generation = Arc::new(AtomicU64::new(0));
        let mut worker = Self {
            agent,
            child,
            permissions: Arc::clone(&permissions),
            stop: None,
            finished: finished_rx,
            owner: owner.to_path_buf(),
            prompts: prompts.clone(),
            journal,
            generation: Arc::clone(&generation),
            closing: Arc::clone(&closing),
            done: false,
            exit_seen: false,
            caps: capabilities(if agent == AGY_AGENT {
                HarnessId::Agy
            } else {
                HarnessId::Cursor
            }),
            markers: markers.clone(),
            state: HeadlessState::default(),
            events: Arc::clone(&events),
        };
        worker.write_owner(&agent)?;
        let child_id = worker.child.id();
        locked(starting).insert(id.to_string(), worker.child.id());
        let server = serde_json::from_value::<McpServer>(acp_mcp_server(
            &request.exe,
            id,
            &request.inbox,
            owner,
            request.snapshots.then_some(cwd.as_path()),
        ))
        .map_err(|e| format!("could not build the worker MCP entry: {e}"))?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        worker.stop = Some(stop_tx);
        let inbox_path = request.inbox.clone();
        let inbox = Inbox::new(inbox_path.clone());
        let worker_id = id.to_string();
        let changes = request
            .inbox
            .parent()
            .filter(|_| request.snapshots)
            .and_then(|store| {
                WorkerChanges::open(&WorkerSite {
                    store: store.to_path_buf(),
                    id: id.to_string(),
                    worktree: cwd.clone(),
                })
            });
        let results = results.to_path_buf();
        let turns_permissions = Arc::clone(&permissions);
        let heartbeat = Arc::clone(&permissions.heartbeat);
        heartbeat.clear();
        let on_request = Arc::clone(&heartbeat);
        let on_notification = Arc::clone(&heartbeat);
        let in_turns = Arc::clone(&heartbeat);
        let ended = Arc::clone(&closing);
        let opened = Arc::new(AtomicBool::new(false));
        let was_opened = Arc::clone(&opened);
        let session_model = request.model.clone();
        let updates = Arc::new(Mutex::new(None));
        let on_update = Arc::clone(&updates);
        thread::spawn(move || {
            let failed = ready_tx.clone();
            let transport = transport::streams(stdin, stdout, log);
            let outcome = block_on(
                Client
                    .builder()
                    .on_receive_request(
                        async move |request: RawPermissionRequest, responder, _cx| {
                            on_request.touch();
                            permissions.push(&request, responder);
                            Ok(())
                        },
                        agent_client_protocol::on_receive_request!(),
                    )
                    .on_receive_notification(
                        async move |notification: SessionNotification, _cx| {
                            on_notification.touch();
                            if agent == AGY_AGENT && locked(&on_update).is_some() {
                                if let Ok(update) = serde_json::to_value(&notification.update) {
                                    agy_settings::observe(
                                        &on_update,
                                        &notification.session_id.to_string(),
                                        &update,
                                    );
                                }
                            }
                            Ok(())
                        },
                        agent_client_protocol::on_receive_notification!(),
                    )
                    .connect_with(transport, |connection: ConnectionTo<Agent>| async move {
                        let init = match initialize(&connection, &agent).await? {
                            Ok(init) => init,
                            Err(message) => {
                                ready_tx.send(Err(message)).ok();
                                return Ok(());
                            }
                        };
                        if agent == CURSOR_AGENT {
                            if let Err(e) = authenticate(&connection, &agent, &init).await {
                                ready_tx
                                    .send(Err(session_error(&agent, i32::from(e.code), &e.message)))
                                    .ok();
                                return Ok(());
                            }
                        }
                        let opening = Opening {
                            agent,
                            launch,
                            cwd,
                            server,
                            worker_id: worker_id.clone(),
                            model: session_model,
                            permission_mode,
                            effort,
                            updates,
                            inbox: inbox_path,
                        };
                        let loads = init.agent_capabilities.load_session;
                        match open_session(&connection, &opening, loads).await {
                            Ok((session, settings)) => {
                                if let Some(ref settings) = settings {
                                    if let Err(message) =
                                        recovery::save(&opening, &session, settings)
                                    {
                                        connection
                                            .send_request(CloseSessionRequest::new(session))
                                            .block_task()
                                            .await
                                            .ok();
                                        ready_tx.send(Err(message)).ok();
                                        return Ok(());
                                    }
                                }
                                opened.store(true, Ordering::SeqCst);
                                ready_tx.send(Ok(())).ok();
                                let turns = Turns {
                                    permissions: Arc::clone(&turns_permissions),
                                    agent,
                                    session,
                                    worker_id,
                                    model: settings.map(|settings| settings.model),
                                    store: opening.inbox.parent().map(Path::to_path_buf),
                                    markers,
                                    results,
                                    inbox,
                                    closing,
                                    changes,
                                    generation,
                                    group: child_id,
                                    heartbeat: in_turns,
                                    events,
                                };
                                run_turns(&connection, &turns, prompt_rx, stop_rx).await;
                                if agent == AGY_AGENT {
                                    connection
                                        .send_request(CloseSessionRequest::new(
                                            turns.session.clone(),
                                        ))
                                        .block_task()
                                        .await
                                        .ok();
                                }
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

    fn write_owner(&self, agent: &AcpAgent) -> Result<(), String> {
        if *agent == AGY_AGENT {
            let _lock = recovery::lock(&self.owner).map_err(|e| e.to_string())?;
            if !recovery::owned(&self.owner) {
                return Err("agy owner claim was lost".into());
            }
            write_owner_record(&self.owner, self.child.id(), agent.name)
        } else {
            write_owner_record(&self.owner, self.child.id(), agent.name)
        }
    }

    fn reap_exit(&mut self) {
        if self.done || self.exit_seen {
            return;
        }
        if !exited_unreaped(&self.child) {
            return;
        }
        signal_group(&self.child);
        if let Ok(Some(status)) = self.child.try_wait() {
            self.exit_seen = true;
            unregister_group(self.child.id());
            if !self.closing.load(Ordering::SeqCst) {
                self.permissions
                    .heartbeat
                    .gone(&Scope::Worker, &exit_reason(status, &self.agent));
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
        if !self.exit_seen {
            stop_child(&mut self.child);
        }
        if self.agent != AGY_AGENT {
            fs::remove_file(&self.owner).ok();
        }
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
                let item = if let Some(dir) = &self.journal {
                    Inbound::TrackedPrompt(text, journal::record(dir)?)
                } else {
                    Inbound::Prompt(text)
                };
                self.prompts
                    .unbounded_send(item)
                    .map_err(|_| format!("{} agent is not accepting prompts", self.agent.name))
            }
            WorkerCommand::AnswerPermission(answer) => self.permissions.answer(&answer),
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
        std::mem::take(&mut *locked(&self.events))
    }
}

impl Drop for AcpWorker {
    fn drop(&mut self) {
        self.terminate();
    }
}

async fn authenticate(
    connection: &ConnectionTo<Agent>,
    agent: &AcpAgent,
    init: &InitializeResponse,
) -> Result<(), agent_client_protocol::Error> {
    let advertised: Vec<&str> = init
        .auth_methods
        .iter()
        .map(|method| method.id().0.as_ref())
        .collect();
    let Some(method) = auth_method(agent, &advertised) else {
        return Ok(());
    };
    connection
        .send_request(AuthenticateRequest::new(method.to_string()))
        .block_task()
        .await
        .map(drop)
}

pub struct CursorHost {
    agy: bool,
    store_dir: PathBuf,
    workers: Mutex<HashMap<String, CursorWorker>>,
    starting: Mutex<HashMap<String, u32>>,
    closed: AtomicBool,
}

impl CursorHost {
    pub fn new(store_dir: PathBuf) -> Arc<Self> {
        if let Err(error) = model::repair_saved_cursor_default() {
            warn(format!("cursor model default was not restored: {error}"));
        }
        for id in request_ids(&store_dir, CURSOR_OWNER_DIR) {
            sweep_owner(&HandoffFiles::cursor(&store_dir, &id));
        }
        fs::remove_dir_all(store_dir.join(CURSOR_INTERRUPT_RESULT_DIR)).ok();
        Arc::new(Self {
            agy: false,
            store_dir,
            workers: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    pub fn agy(store_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            agy: true,
            store_dir,
            workers: Mutex::new(HashMap::new()),
            starting: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    fn workers(&self) -> MutexGuard<'_, HashMap<String, CursorWorker>> {
        locked(&self.workers)
    }

    fn files(&self, id: &str) -> HandoffFiles {
        if self.agy {
            HandoffFiles::agy(&self.store_dir, id)
        } else {
            HandoffFiles::cursor(&self.store_dir, id)
        }
    }

    fn handles(&self, id: &str) -> bool {
        !self.agy || !self.files(id).owner.exists() || recovery::owned(&self.files(id).owner)
    }

    fn reap_exits(&self) {
        for worker in self.workers().values_mut() {
            worker.reap_exit();
            worker.apply_events();
        }
    }

    fn poll(&self) {
        self.reap_exits();
        for id in request_ids(&self.store_dir, self.dir(CURSOR_CLOSE_DIR, "agy-close")) {
            if self.agy || self.handles(&id) {
                self.close(&id);
            }
        }
        for id in request_ids(&self.store_dir, self.dir(CURSOR_ANSWER_DIR, "agy-answer")) {
            if self.handles(&id) {
                self.answer(&id);
            }
        }
        for id in request_ids(&self.store_dir, self.dir(CURSOR_PROMPT_DIR, "agy-prompt")) {
            if !self.handles(&id) || (self.agy && !self.workers().contains_key(&id)) {
                continue;
            }
            self.deliver_queued(
                &id,
                |files| &files.prompt,
                |worker, text, _| worker.send(WorkerCommand::Prompt(text)),
                |_, entry| mark_prompt_lost(entry),
            );
        }
        for id in request_ids(
            &self.store_dir,
            self.dir(CURSOR_INTERRUPT_DIR, "agy-interrupt"),
        ) {
            if !self.handles(&id) || (self.agy && !self.workers().contains_key(&id)) {
                continue;
            }
            self.deliver_queued(
                &id,
                |files| &files.interrupt,
                |worker, text, entry| worker.interrupt(&text, entry),
                |files, entry| report_interrupt_gone(&files.interrupt_result, entry),
            );
        }
        for id in request_ids(&self.store_dir, self.dir(CURSOR_SPAWN_DIR, "agy-spawn")) {
            if self.handles(&id) {
                self.spawn(&id);
            }
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let mut drained: Vec<(String, CursorWorker)> = self.workers().drain().collect();
        for (_, worker) in &mut drained {
            worker.begin_stop();
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        for (id, worker) in &mut drained {
            worker.finish_stop(deadline);
            self.forget(id, false);
        }
        let starting: Vec<(String, u32)> = locked(&self.starting).drain().collect();
        if starting.is_empty() {
            return;
        }
        let groups: Vec<u32> = starting.iter().map(|(_, group)| *group).collect();
        stop_groups(&groups, SHUTDOWN_GRACE, || true);
        for (id, _) in &starting {
            self.forget(id, false);
        }
    }

    fn forget(&self, id: &str, close: bool) {
        let files = self.files(id);
        let _lock = if self.agy {
            let Ok(lock) = recovery::lock(&files.owner) else {
                return;
            };
            if !recovery::owned(&files.owner) {
                return;
            }
            Some(lock)
        } else {
            None
        };
        self.cleanup(id, &files, close);
    }

    fn answer(&self, id: &str) {
        take_answer(&self.files(id), |text| {
            PermissionAnswer::decode(text).and_then(|answer| {
                self.workers()
                    .get_mut(id)
                    .ok_or_else(|| format!("worker has no running {} agent", CURSOR_AGENT.name))
                    .and_then(|worker| worker.send(WorkerCommand::AnswerPermission(answer)))
            })
        });
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
        if self.agy && !recovery::claim(&files.owner) {
            return;
        }
        let started = if self.agy {
            self.launch(id, &request, Launch::Fresh)
                .map(|worker| CursorWorker::Acp(Box::new(worker)))
        } else {
            PrintWorker::start(id, &request, &files, None, cursor_permissions())
                .map(CursorWorker::Print)
        };
        match started {
            Ok(worker) => self.adopt(id, worker, self.agy.then(|| task_prompt(&request.task))),
            Err(message) => {
                if self.agy {
                    self.forget(id, true);
                } else {
                    fs::remove_file(&files.session).ok();
                }
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
                agent: if self.agy { AGY_AGENT } else { CURSOR_AGENT },
                id,
                owner: &self.files(id).owner,
                results: &self.files(id).interrupt_result,
                markers: WorkerMarkers::new(&self.store_dir, id),
                starting: &self.starting,
            },
        );
        locked(&self.starting).remove(id);
        started
    }

    fn resume_all(&self) {
        for id in request_ids(&self.store_dir, self.dir(CURSOR_SESSION_DIR, "agy-session")) {
            self.resume(&id);
        }
    }

    fn resume(&self, id: &str) {
        let files = self.files(id);
        if files.owner.exists() && !self.agy {
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
        if let Resume::Skip(message) = recovery::resume(
            if self.agy { &AGY_AGENT } else { &CURSOR_AGENT },
            id,
            &record,
            &self.store_dir,
            true,
        ) {
            warn(message);
            return;
        }
        if self.agy {
            let Ok(_lock) = recovery::lock(&files.owner) else {
                return;
            };
            if files.owner.exists() && !crate::cursor_owner::reclaim_agy(&files.owner) {
                return;
            }
            if let Err(error) = journal::recover(&self.store_dir, id) {
                warn(format!(
                    "agy session {id} not resumed: could not report unfinished work: {error}"
                ));
                return;
            }
        }
        let Ok(exe) = std::env::current_exe() else {
            warn(format!(
                "{} session {id} not resumed: could not find the ralph executable",
                CURSOR_AGENT.name
            ));
            return;
        };
        if !record.cwd.is_dir() {
            let message = format!(
                "{} session {id} not resumed: its worktree {} is gone",
                if self.agy {
                    AGY_AGENT.name
                } else {
                    CURSOR_AGENT.name
                },
                record.cwd.display()
            );
            warn(message.clone());
            if self.agy {
                write_file(&files.error, &message).ok();
            }
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
        if self.agy && !recovery::claim_saved(&files, &record) {
            return;
        }
        let started = if !self.agy && record.permissions.is_some() {
            let Some(mode) = record.permissions else {
                return;
            };
            PrintWorker::start(id, &request, &files, Some(&record), mode).map(CursorWorker::Print)
        } else {
            let launch = Launch::Resume {
                record,
                store: self.store_dir.clone(),
            };
            self.launch(id, &request, launch)
                .map(|worker| CursorWorker::Acp(Box::new(worker)))
        };
        match started {
            Ok(worker) => self.adopt_resumed(id, worker),
            Err(message) => {
                if self.agy {
                    recovery::remove_owned(&files.owner);
                }
                warn(message.clone());
                write_file(&files.error, &message).ok();
            }
        }
    }

    fn adopt_resumed(&self, id: &str, worker: CursorWorker) {
        let ready = ready_path(&self.store_dir, id);
        if !self.closed.load(Ordering::SeqCst) && write_file(&ready, "").is_ok() {
            self.workers().insert(id.to_string(), worker);
        } else {
            drop(worker);
            if self.agy {
                self.forget(id, false);
            }
            if !self.agy {
                fs::remove_file(ready).ok();
            }
        }
    }
    fn adopt(&self, id: &str, mut worker: CursorWorker, task: Option<String>) {
        let files = self.files(id);
        let claimed = !self.closed.load(Ordering::SeqCst)
            && (!self.agy || recovery::owned(&files.owner))
            && fs::remove_file(&files.spawn).is_ok()
            && write_file(&ready_path(&self.store_dir, id), "").is_ok();
        if claimed {
            if let Some(task) = task {
                if let Err(error) = worker.send(WorkerCommand::Prompt(task)) {
                    drop(worker);
                    if self.agy {
                        self.forget(id, true);
                    } else {
                        fs::remove_file(ready_path(&self.store_dir, id)).ok();
                    }
                    self.fail(id, &error);
                    return;
                }
            }
            self.workers().insert(id.to_string(), worker);
        } else {
            drop(worker);
            if self.agy {
                self.forget(id, true);
            } else {
                fs::remove_file(ready_path(&self.store_dir, id)).ok();
                fs::remove_file(&files.session).ok();
            }
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
