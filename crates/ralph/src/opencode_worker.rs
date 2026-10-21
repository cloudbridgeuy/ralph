use crate::acp_worker::{apply_effect, push_event, Events};
use crate::config::AppConfig;
use crate::cursor_owner::{now_secs, process_start, write_file};
use crate::transport::http::HttpSse;
use crate::transport::stdio::stop_child;
use crate::warn::warn;
use crate::worker_changes::WorkerChanges;
use crate::worker_mcp::Inbox;
use crate::workers::headless::{HeadlessHost, Heartbeat};
use ralph_core::snapshot::{snapshots_enabled, WorkerSite};
use ralph_core::worker::Scope;
use ralph_core::worker::{
    advance, descendants, hex_encode, opencode_failure_report, opencode_serve_command,
    opencode_server_log, opencode_server_record_path, opencode_session_path, parse_event, parse_ps,
    reconcile, reconnect_delay, reduce, running_sessions, server_record_fate, server_record_pid,
    server_record_usable, session_activity, session_missing, session_update, spawn_failure,
    stale_server_group, start_failure, turn_result, ApiData, Effect, HandoffFiles, HeadlessState,
    HeadlessWorker, OpencodeRecord, PermissionAsked, RecordFate, ServerEvent, ServerRecord,
    ServerRecordFacts, SessionMessage, SessionSnapshot, SessionUpdate, TurnFacts, TurnState,
    WorkerCommand, WorkerEvent, WorkerMarkers, OPENCODE_INTERRUPT_RESULT_DIR, OPENCODE_OWNER_DIR,
    OPENCODE_PASSWORD_ENV, OPENCODE_SESSION_DIR, RECONNECT_DELAY, SILENCE_TIMEOUT,
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

mod requests;
mod session;

pub(crate) use requests::entry_ids;
use session::OpencodeSession;

const HEALTH_PATH: &str = "/api/info";
const ACTIVE_PATH: &str = "/api/session/active";
const EVENTS_PATH: &str = "/api/event";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(20);
const HEALTH_POLL: Duration = Duration::from_millis(100);
const HOST_POLL: Duration = Duration::from_millis(500);
const SNAPSHOT_INTERVAL: Duration = Duration::from_secs(3);
const SWEEP_GRACE: Duration = Duration::from_millis(500);
const MESSAGE_LIMIT: u32 = 20;
const START_ATTEMPTS: u32 = 5;

struct Server {
    child: Child,
    api: HttpSse,
    tree: Vec<u32>,
    record: PathBuf,
}

impl Server {
    fn launch(store: &Path, closed: &AtomicBool) -> Result<Self, String> {
        let port = free_port()?;
        let password = random_password()?;
        let log = open_log(store)?;
        let command = opencode_serve_command(port);
        let (program, args) = command.split_first().ok_or("empty opencode command")?;
        let child = match Command::new(program)
            .args(args)
            .env(OPENCODE_PASSWORD_ENV, &password)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .process_group(0)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => return Err(spawn_failure(program, &error)),
        };
        let mut server = Self {
            child,
            api: HttpSse::new(port, &password),
            tree: Vec::new(),
            record: opencode_server_record_path(&std::env::temp_dir(), std::process::id()),
        };
        let recorded = owner_record(port, &password, server.child.id())
            .and_then(|record| write_record(&server.record, &record));
        match recorded.and_then(|()| server.wait_healthy(closed)) {
            Ok(()) => Ok(server),
            Err(message) => stop_failed(server, store, message),
        }
    }

    fn wait_healthy(&mut self, closed: &AtomicBool) -> Result<(), String> {
        let deadline = Instant::now() + HEALTH_TIMEOUT;
        loop {
            if closed.load(Ordering::SeqCst) {
                return Err("the chat is closing".to_string());
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!("the opencode server exited at start: {status}"));
            }
            if self.api.get(HEALTH_PATH).is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the opencode server was not healthy after {}s",
                    HEALTH_TIMEOUT.as_secs()
                ));
            }
            thread::sleep(HEALTH_POLL);
        }
    }

    fn label(&self) -> String {
        format!("pid {}", self.child.id())
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn snapshot(&mut self) {
        self.tree = descendants(&process_table(), self.child.id());
    }

    fn stop(mut self) {
        fs::remove_file(&self.record).ok();
        let group = self.child.id();
        let mut orphans = self.tree.clone();
        for pid in descendants(&process_table(), group) {
            if !orphans.contains(&pid) {
                orphans.push(pid);
            }
        }
        stop_child(&mut self.child);
        sweep(&orphans);
    }
}

fn owner_record(port: u16, password: &str, server_pid: u32) -> Result<ServerRecord, String> {
    let pid = std::process::id();
    let start = process_start(pid).ok_or("could not read the chat start time")?;
    Ok(ServerRecord {
        port,
        password: password.to_string(),
        pid,
        start,
        server_pid: Some(server_pid),
        server_start: process_start(server_pid),
    })
}

fn stop_failed(server: Server, store: &Path, message: String) -> Result<Server, String> {
    server.stop();
    let log = fs::read_to_string(opencode_server_log(store)).unwrap_or_default();
    Err(start_failure(&message, &log))
}

fn write_record(path: &Path, record: &ServerRecord) -> Result<(), String> {
    fs::remove_file(path).ok();
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(record.render().as_bytes()))
        .map_err(|e| format!("could not write the opencode server record: {e}"))
}

pub fn running_opencode_sessions() -> Option<HashSet<String>> {
    let pid = std::process::id();
    let path = opencode_server_record_path(&std::env::temp_dir(), pid);
    let record = ServerRecord::parse(&fs::read_to_string(&path).ok()?)?;
    if !server_record_usable(&record, pid, process_start(pid).as_deref()) {
        fs::remove_file(&path).ok();
        return None;
    }
    let body = HttpSse::new(record.port, &record.password)
        .get(ACTIVE_PATH)
        .ok()?;
    running_sessions(&body).map(|running| running.into_iter().collect())
}

pub(crate) fn sweep_stale_server_records() {
    let dir = std::env::temp_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let own_pid = std::process::id();
    for name in entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
    {
        let file_pid = server_record_pid(&name);
        let path = dir.join(&name);
        let record = file_pid
            .and_then(|_| fs::read_to_string(&path).ok())
            .and_then(|text| ServerRecord::parse(&text));
        let start_now = file_pid.and_then(process_start);
        let fate = server_record_fate(ServerRecordFacts {
            file_pid,
            own_pid,
            record: record.as_ref(),
            file_pid_start_now: start_now.as_deref(),
        });
        if fate == RecordFate::Delete {
            if let Some(group) = record.as_ref().and_then(stale_group_now) {
                stop_group(group);
            }
            fs::remove_file(&path).ok();
        }
    }
}

fn stale_group_now(record: &ServerRecord) -> Option<u32> {
    let start = record.server_pid.and_then(process_start);
    stale_server_group(record, start.as_deref())
}

fn stop_group(group: u32) {
    let target = format!("-{group}");
    kill("TERM", &target);
    thread::sleep(SWEEP_GRACE);
    if kill("0", &target) {
        kill("KILL", &target);
    }
}

fn kill(signal: &str, target: &str) -> bool {
    Command::new("kill")
        .args([&format!("-{signal}"), "--", target])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn survivors(pids: &[u32]) -> Vec<u32> {
    pids.iter()
        .copied()
        .filter(|pid| kill("0", &pid.to_string()))
        .collect()
}

fn sweep(pids: &[u32]) {
    let found = survivors(pids);
    if found.is_empty() {
        return;
    }
    for pid in &found {
        kill("TERM", &pid.to_string());
    }
    thread::sleep(SWEEP_GRACE);
    for pid in survivors(&found) {
        kill("KILL", &pid.to_string());
    }
}

fn process_table() -> Vec<(u32, u32)> {
    Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| parse_ps(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|e| format!("could not find a free port: {e}"))
}

fn random_password() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|e| format!("could not read random bytes: {e}"))?;
    Ok(hex_encode(&bytes))
}

fn open_log(store: &Path) -> Result<File, String> {
    fs::create_dir_all(store).map_err(|e| e.to_string())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(opencode_server_log(store))
        .map_err(|e| format!("could not open the opencode server log: {e}"))
}

#[derive(Clone)]
struct Target {
    worker: String,
    markers: WorkerMarkers,
    inbox: PathBuf,
    site: Option<WorkerSite>,
}

struct Session {
    target: Target,
    turn: TurnState,
    verified: bool,
    heartbeat: Arc<Heartbeat>,
    state: HeadlessState,
    events: Events,
}

enum Feed {
    Line(String),
    Ended,
}

struct Link {
    feed: mpsc::Receiver<Feed>,
    last: Instant,
}

fn read_lines(reader: impl Read, feed: &Sender<Feed>) {
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if feed.send(Feed::Line(line)).is_err() {
            return;
        }
    }
    feed.send(Feed::Ended).ok();
}

fn load_sessions(store: &Path, snapshots: bool) -> HashMap<String, Session> {
    let ids: Vec<String> = fs::read_dir(store.join(OPENCODE_SESSION_DIR))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !name.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    ids.into_iter()
        .filter_map(|id| {
            let record = fs::read_to_string(opencode_session_path(store, &id))
                .ok()
                .and_then(|text| OpencodeRecord::parse(&text));
            let Some(record) = record else {
                warn(format!(
                    "opencode session {id} not resumed: its record is unreadable"
                ));
                return None;
            };
            let markers = WorkerMarkers::new(store, &id);
            let heartbeat = Heartbeat::new(markers.activity.clone(), markers.gone.clone());
            heartbeat.clear();
            let site = snapshots.then(|| WorkerSite {
                store: store.to_path_buf(),
                id: id.clone(),
                worktree: record.cwd,
            });
            let session = Session {
                target: Target {
                    markers,
                    worker: id,
                    inbox: record.inbox,
                    site,
                },
                turn: TurnState::default(),
                verified: false,
                heartbeat,
                state: HeadlessState::default(),
                events: Events::default(),
            };
            Some((record.session_id, session))
        })
        .collect()
}

pub struct OpencodeHost {
    store_dir: PathBuf,
    server: Mutex<Option<Server>>,
    sessions: Mutex<HashMap<String, Session>>,
    closed: AtomicBool,
}

impl OpencodeHost {
    pub(crate) fn new(store_dir: PathBuf) -> Arc<Self> {
        fs::remove_dir_all(store_dir.join(OPENCODE_OWNER_DIR)).ok();
        fs::remove_dir_all(store_dir.join(OPENCODE_INTERRUPT_RESULT_DIR)).ok();
        Arc::new(Self {
            sessions: Mutex::new(load_sessions(
                &store_dir,
                snapshots_enabled(AppConfig::load().ok().map(|config| config.changes.enabled)),
            )),
            store_dir,
            server: Mutex::new(None),
            closed: AtomicBool::new(false),
        })
    }

    fn server(&self) -> MutexGuard<'_, Option<Server>> {
        self.server.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sessions(&self) -> MutexGuard<'_, HashMap<String, Session>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn api(&self) -> Option<HttpSse> {
        self.server().as_ref().map(|server| server.api.clone())
    }

    fn start_server(&self) -> Result<HttpSse, String> {
        let mut guard = self.server();
        if self.closed() {
            return Err("the chat is closing".to_string());
        }
        if let Some(server) = guard.as_mut() {
            return if server.alive() {
                Ok(server.api.clone())
            } else {
                Err("the opencode server is restarting".to_string())
            };
        }
        let server = Server::launch(&self.store_dir, &self.closed)?;
        let api = server.api.clone();
        *guard = Some(server);
        Ok(api)
    }

    fn take_dead(&self) -> Option<Server> {
        let mut guard = self.server();
        if guard.as_mut().is_some_and(|server| !server.alive()) {
            guard.take()
        } else {
            None
        }
    }

    fn server_alive(&self) -> bool {
        self.server().as_mut().is_some_and(Server::alive)
    }

    fn lose_server(&self) {
        let taken = self.server().take();
        if let Some(server) = taken {
            self.mark_gone(&server.label(), "unreachable");
            server.stop();
            self.lose_sessions();
        }
    }

    fn mark_gone(&self, label: &str, how: &str) {
        if self.closed() {
            return;
        }
        let scope = Scope::Server(label.to_string());
        let reason = format!("{how}; see opencode-server.log");
        for session in self.sessions().values() {
            session.heartbeat.gone(&scope, &reason);
        }
    }

    fn revive_all(&self) {
        for session in self.sessions().values() {
            session.heartbeat.revive();
        }
    }

    fn snapshot_tree(&self) {
        if let Some(server) = self.server().as_mut() {
            server.snapshot();
        }
    }

    fn lose_sessions(&self) {
        let lost: Vec<(String, Target, Vec<Effect>)> = self
            .sessions()
            .iter_mut()
            .map(|(id, session)| {
                (
                    id.clone(),
                    session.target.clone(),
                    session.turn.server_lost(),
                )
            })
            .collect();
        for (id, target, effects) in lost {
            self.run(&id, &target, effects);
        }
    }

    fn apply(&self, session_id: &str, update: SessionUpdate) {
        let planned = {
            let mut sessions = self.sessions();
            sessions.get_mut(session_id).map(|session| {
                let facts = TurnFacts {
                    reported: session.target.markers.reported.exists(),
                    closing: self.closed(),
                };
                (
                    session.target.clone(),
                    advance(&mut session.turn, update, facts),
                )
            })
        };
        if let Some((target, effects)) = planned {
            self.run(session_id, &target, effects);
        }
    }

    fn worker(&self, id: &str, session_id: &str) -> OpencodeSession {
        let (front, events) = self
            .sessions()
            .get(session_id)
            .map(|session| {
                (
                    session.turn.blocked.front().cloned(),
                    session.events.clone(),
                )
            })
            .unwrap_or_default();
        OpencodeSession::new(id, session_id, self.api(), front, events)
    }

    fn apply_events(&self, id: &str, session_id: &str) -> Vec<WorkerEvent> {
        let events = self.worker(id, session_id).poll();
        if let Some(session) = self.sessions().get_mut(session_id) {
            for event in &events {
                let (state, effects) = reduce(&session.state, event);
                session.state = state;
                for effect in &effects {
                    apply_effect(&session.target.markers, effect);
                }
            }
        }
        events
    }

    fn run(&self, session_id: &str, target: &Target, effects: Vec<Effect>) {
        let markers = &target.markers;
        let worker = self.worker(&target.worker, session_id);
        for effect in effects {
            match effect {
                Effect::Ready => push_event(&worker.events, WorkerEvent::Ready),
                Effect::ClearWaiting => push_event(&worker.events, WorkerEvent::TurnStarted),
                Effect::Publish(Some(content)) => {
                    push_event(&worker.events, WorkerEvent::PermissionAsked(content));
                }
                Effect::Publish(None) => push_event(&worker.events, WorkerEvent::PermissionSettled),
                Effect::ClearReported => {
                    fs::remove_file(&markers.reported).ok();
                }
                Effect::Unreported => {
                    write_file(&markers.unreported, "").ok();
                    write_file(&markers.waiting, "").ok();
                }
                Effect::Problem(problem) => post_problem(target, &problem),
                Effect::Send(text) => self.send(session_id, target, &text),
                Effect::Snapshot => {
                    if let Some(changes) = target.site.as_ref().and_then(WorkerChanges::open) {
                        changes.snapshot_turn_or_warn();
                    }
                }
            }
        }
        self.apply_events(&target.worker, session_id);
    }

    fn send(&self, session_id: &str, target: &Target, text: &str) {
        let result = self
            .worker(&target.worker, session_id)
            .send(WorkerCommand::Prompt(text.to_string()));
        if let Err(message) = result {
            if let Some(session) = self.sessions().get_mut(session_id) {
                session.turn.in_flight = false;
            }
            post_problem(target, &format!("could not send the prompt: {message}"));
        }
    }

    fn route(&self, event: &ServerEvent) {
        let Some(session_id) = event.session_id() else {
            return;
        };
        let update = session_update(event);
        let heartbeat = self
            .sessions()
            .get(session_id)
            .map(|session| Arc::clone(&session.heartbeat));
        if let Some(heartbeat) = heartbeat {
            match &update {
                Some(SessionUpdate::TurnStarted) => heartbeat.set_running(true),
                Some(SessionUpdate::TurnEnded(_)) => heartbeat.set_running(false),
                _ => {
                    if let Some(WorkerEvent::Activity { at }) = session_activity(event, now_secs())
                    {
                        heartbeat.touch_at(at);
                    }
                }
            }
        }
        if let Some(update) = update {
            self.apply(session_id, update);
        }
    }

    fn snapshot_of(
        api: &HttpSse,
        session_id: &str,
        state: &TurnState,
        running: &[String],
    ) -> Result<SessionSnapshot, String> {
        let active = running.iter().any(|id| id == session_id);
        let pending: ApiData<Vec<PermissionAsked>> =
            serde_json::from_str(&api.get(&format!("/api/session/{session_id}/permission"))?)
                .map_err(|e| e.to_string())?;
        let result = if state.in_flight && !active {
            let messages: ApiData<Vec<SessionMessage>> = serde_json::from_str(&api.get(
                &format!("/api/session/{session_id}/message?limit={MESSAGE_LIMIT}"),
            )?)
            .map_err(|e| e.to_string())?;
            turn_result(&messages.data, active)
        } else {
            None
        };
        Ok(SessionSnapshot {
            active,
            pending: pending.data,
            result,
        })
    }

    fn reconcile_all(&self) {
        let Some(api) = self.api() else { return };
        let running = api
            .get(ACTIVE_PATH)
            .ok()
            .and_then(|body| running_sessions(&body));
        let Some(running) = running else {
            warn("could not read the active opencode sessions");
            return;
        };
        self.verify_resumed(&api);
        let known: Vec<(String, TurnState)> = self
            .sessions()
            .iter()
            .filter(|(_, session)| session.verified)
            .map(|(id, session)| (id.clone(), session.turn.clone()))
            .collect();
        for (id, state) in known {
            match Self::snapshot_of(&api, &id, &state, &running) {
                Ok(snapshot) => {
                    for update in reconcile(&state, &snapshot) {
                        self.apply(&id, update);
                    }
                }
                Err(message) => warn(format!(
                    "could not rebuild opencode session {id}: {message}"
                )),
            }
        }
    }

    fn verify_resumed(&self, api: &HttpSse) {
        let pending: Vec<(String, Target)> = self
            .sessions()
            .iter()
            .filter(|(_, session)| !session.verified)
            .map(|(id, session)| (id.clone(), session.target.clone()))
            .collect();
        for (session_id, target) in pending {
            match api.get(&format!("/api/session/{session_id}")) {
                Ok(_) => {
                    if let Some(session) = self.sessions().get_mut(&session_id) {
                        session.verified = true;
                    }
                    write_file(&self.files(&target.worker).owner, "").ok();
                }
                Err(message) if session_missing(&message) => {
                    self.sessions().remove(&session_id);
                    let files = self.files(&target.worker);
                    fs::remove_file(&files.session).ok();
                    write_file(
                        &files.error,
                        &format!("opencode session {session_id} is gone; the worker cannot resume"),
                    )
                    .ok();
                    warn(format!(
                        "opencode session {session_id} not resumed: it is gone"
                    ));
                }
                Err(message) => warn(format!(
                    "could not verify opencode session {session_id}: {message}"
                )),
            }
        }
    }

    fn files(&self, id: &str) -> HandoffFiles {
        HandoffFiles::opencode(&self.store_dir, id)
    }

    fn connect(&self) -> Result<Link, String> {
        let api = self
            .api()
            .ok_or_else(|| "the opencode server is not running".to_string())?;
        let reader = api.events(EVENTS_PATH)?;
        let (sender, feed) = mpsc::channel();
        thread::spawn(move || read_lines(reader, &sender));
        Ok(Link {
            feed,
            last: Instant::now(),
        })
    }

    fn reconnect(&self, first: Duration) -> Option<Link> {
        let mut attempt = 0;
        thread::sleep(first);
        loop {
            if self.closed() || !self.server_alive() {
                return None;
            }
            match self.connect() {
                Ok(link) => return Some(link),
                Err(message) => {
                    let delay = reconnect_delay(attempt)?;
                    warn(format!("opencode event stream unavailable: {message}"));
                    attempt += 1;
                    thread::sleep(delay);
                }
            }
        }
    }

    fn supervise(&self) {
        let mut link: Option<Link> = None;
        let mut delay = Duration::ZERO;
        let mut start_failures = 0;
        let mut next_snapshot = Instant::now();
        while !self.closed() {
            if self.sessions().is_empty() && self.server().is_none() {
                thread::sleep(HOST_POLL);
                continue;
            }
            if let Some(dead) = self.take_dead() {
                warn("the opencode server stopped; restarting it");
                self.mark_gone(&dead.label(), "process exited");
                dead.stop();
                self.lose_sessions();
                link = None;
            }
            if self.server().is_none() {
                match self.start_server() {
                    Ok(_) => {
                        self.revive_all();
                        start_failures = 0;
                        delay = Duration::ZERO;
                    }
                    Err(message) => {
                        warn(message);
                        start_failures += 1;
                        if start_failures >= START_ATTEMPTS {
                            warn("giving up on the opencode server");
                            return;
                        }
                        thread::sleep(RECONNECT_DELAY);
                        continue;
                    }
                }
            }
            let Some(active) = link.as_mut() else {
                match self.reconnect(delay) {
                    Some(new) => {
                        self.reconcile_all();
                        link = Some(new);
                        delay = RECONNECT_DELAY;
                    }
                    None => {
                        if !self.closed() {
                            warn("the opencode server is unreachable; restarting it");
                            self.lose_server();
                        }
                    }
                }
                continue;
            };
            let dropped = match active.feed.recv_timeout(HOST_POLL) {
                Ok(Feed::Line(line)) => {
                    active.last = Instant::now();
                    if let Some(event) = parse_event(&line) {
                        self.route(&event);
                    }
                    false
                }
                Ok(Feed::Ended) | Err(RecvTimeoutError::Disconnected) => true,
                Err(RecvTimeoutError::Timeout) => active.last.elapsed() >= SILENCE_TIMEOUT,
            };
            if dropped {
                link = None;
                delay = RECONNECT_DELAY;
            }
            if Instant::now() >= next_snapshot {
                self.snapshot_tree();
                next_snapshot = Instant::now() + SNAPSHOT_INTERVAL;
            }
        }
    }

    fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let taken = self.server().take();
        if let Some(server) = taken {
            server.stop();
        }
    }
}

fn post_prompt(api: &HttpSse, session_id: &str, text: &str) -> Result<(), String> {
    let body = json!({ "text": text }).to_string();
    api.post_retrying(&format!("/api/session/{session_id}/prompt"), &body)
        .map(|_| ())
}

fn post_problem(target: &Target, problem: &str) {
    let at = chrono::Utc::now().to_rfc3339();
    let report = opencode_failure_report(&target.worker, &at, problem);
    Inbox::new(target.inbox.clone()).post(&report, 0).ok();
}

impl HeadlessHost for OpencodeHost {
    fn resume(&self) {
        self.supervise();
    }

    fn poll(&self) -> bool {
        if self.closed() {
            return false;
        }
        OpencodeHost::poll(self);
        true
    }

    fn shutdown(&self) {
        OpencodeHost::shutdown(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(session_id: &str, inbox: &str) -> String {
        OpencodeRecord {
            session_id: session_id.to_string(),
            cwd: PathBuf::from("/w"),
            model: "github-copilot/claude-haiku-4.5".to_string(),
            inbox: PathBuf::from(inbox),
        }
        .render()
    }

    #[test]
    fn load_sessions_keys_by_session_id_and_skips_bad_records() {
        let store = tempfile::tempdir().unwrap();
        let dir = store.path().join(OPENCODE_SESSION_DIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("GUZ-1"), record("ses_one", "/inbox")).unwrap();
        fs::write(dir.join("GUZ-2"), "garbage").unwrap();
        fs::write(dir.join(".tmp"), record("ses_tmp", "/inbox")).unwrap();
        let sessions = load_sessions(store.path(), true);
        assert_eq!(sessions.len(), 1);
        let session = &sessions["ses_one"];
        assert_eq!(session.target.worker, "GUZ-1");
        assert_eq!(session.target.inbox, PathBuf::from("/inbox"));
        assert_eq!(
            session.target.markers,
            WorkerMarkers::new(store.path(), "GUZ-1")
        );
        assert_eq!(
            session.target.site,
            Some(WorkerSite {
                store: store.path().to_path_buf(),
                id: "GUZ-1".to_string(),
                worktree: PathBuf::from("/w"),
            })
        );
        assert_eq!(session.turn, TurnState::default());
        let off = load_sessions(store.path(), false);
        assert_eq!(off["ses_one"].target.site, None);
    }

    #[test]
    fn load_sessions_is_empty_without_records() {
        let store = tempfile::tempdir().unwrap();
        assert!(load_sessions(store.path(), true).is_empty());
    }

    #[test]
    fn free_port_returns_an_assigned_port() {
        assert!(free_port().unwrap() > 0);
    }

    #[test]
    fn random_password_is_32_hex_chars_and_fresh() {
        let first = random_password().unwrap();
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, random_password().unwrap());
    }

    #[test]
    fn read_lines_forwards_every_line_then_ends() {
        let (sender, feed) = mpsc::channel();
        read_lines("data: a\n\n: heartbeat\n".as_bytes(), &sender);
        let lines: Vec<Option<String>> = feed
            .try_iter()
            .map(|item| match item {
                Feed::Line(line) => Some(line),
                Feed::Ended => None,
            })
            .collect();
        assert_eq!(
            lines,
            vec![
                Some("data: a".to_string()),
                Some(String::new()),
                Some(": heartbeat".to_string()),
                None
            ]
        );
    }

    #[test]
    fn survivors_keep_only_live_pids() {
        let own = std::process::id();
        assert_eq!(survivors(&[own, u32::MAX - 1]), vec![own]);
    }
}
