use crate::warn::warn;
use crate::watches::Watches;
use agent_client_protocol::schema::v1::{InitializeRequest, McpServer, NewSessionRequest};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use futures::channel::oneshot;
use futures::executor::block_on;
use ralph_core::worker::{
    acp_mcp_server, check_protocol, ready_path, request_stale, session_error, version_warning,
    CursorFiles, SpawnRequest, CURSOR_CLOSE_DIR, CURSOR_SPAWN_DIR, HANDOFF_POLL,
};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const AGENT_COMMAND: &str = "agent";
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const TERM_GRACE: Duration = Duration::from_secs(3);
const TERM_POLL: Duration = Duration::from_millis(20);

pub struct AcpWorker {
    child: Child,
    stop: Option<oneshot::Sender<()>>,
}

impl AcpWorker {
    pub fn start(request: &SpawnRequest, id: &str) -> Result<Self, String> {
        if let Some(warning) = agent_version_warning() {
            warn(warning);
        }
        let mut child = Command::new(AGENT_COMMAND)
            .arg("acp")
            .current_dir(&request.worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start `{AGENT_COMMAND} acp`: {e}"))?;
        let streams = child.stdin.take().zip(child.stdout.take());
        let mut worker = Self { child, stop: None };
        let (stdin, stdout) = streams.ok_or("cursor agent has no stdio pipes")?;
        let server =
            serde_json::from_value::<McpServer>(acp_mcp_server(&request.exe, id, &request.inbox))
                .map_err(|e| format!("could not build the worker MCP entry: {e}"))?;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        worker.stop = Some(stop_tx);
        let cwd = request.worktree.clone();
        thread::spawn(move || {
            let failed = ready_tx.clone();
            let transport = ByteStreams::new(
                blocking::Unblock::new(stdin),
                blocking::Unblock::new(stdout),
            );
            let outcome = block_on(Client.builder().connect_with(
                transport,
                |connection: ConnectionTo<Agent>| async move {
                    let init = connection
                        .send_request(InitializeRequest::new(ProtocolVersion::V1))
                        .block_task()
                        .await?;
                    if let Err(message) = check_protocol(init.protocol_version.as_u16()) {
                        ready_tx.send(Err(message)).ok();
                        return Ok(());
                    }
                    let session = connection
                        .send_request(NewSessionRequest::new(cwd).mcp_servers(vec![server]))
                        .block_task()
                        .await;
                    match session {
                        Ok(_) => {
                            ready_tx.send(Ok(())).ok();
                            stop_rx.await.ok();
                        }
                        Err(e) => {
                            ready_tx
                                .send(Err(session_error(i32::from(e.code), &e.message)))
                                .ok();
                        }
                    }
                    Ok(())
                },
            ));
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

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn terminate(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        if let Some(stop) = self.stop.take() {
            stop.send(()).ok();
        }
        Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok();
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            thread::sleep(TERM_POLL);
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            self.child.kill().ok();
        }
        self.child.wait().ok();
    }
}

impl Drop for AcpWorker {
    fn drop(&mut self) {
        self.terminate();
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
    closed: AtomicBool,
}

impl CursorHost {
    pub fn new(store_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            store_dir,
            workers: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    fn workers(&self) -> MutexGuard<'_, HashMap<String, AcpWorker>> {
        self.workers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn files(&self, id: &str) -> CursorFiles {
        CursorFiles::new(&self.store_dir, id)
    }

    pub fn poll(&self) {
        for id in request_ids(&self.store_dir, CURSOR_CLOSE_DIR) {
            self.close(&id);
        }
        for id in request_ids(&self.store_dir, CURSOR_SPAWN_DIR) {
            self.spawn(&id);
        }
    }

    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let drained: Vec<(String, AcpWorker)> = self.workers().drain().collect();
        for (id, mut worker) in drained {
            worker.terminate();
            self.forget(&id);
        }
    }

    fn forget(&self, id: &str) {
        let files = self.files(id);
        fs::remove_file(&files.owner).ok();
        fs::remove_file(ready_path(&self.store_dir, id)).ok();
    }

    fn close(&self, id: &str) {
        let removed = self.workers().remove(id);
        if let Some(mut worker) = removed {
            worker.terminate();
        }
        self.forget(id);
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
        match AcpWorker::start(&request, id) {
            Ok(worker) => self.adopt(id, worker),
            Err(message) => self.fail(id, &message),
        }
    }

    fn adopt(&self, id: &str, worker: AcpWorker) {
        let files = self.files(id);
        let claimed = !self.closed.load(Ordering::SeqCst)
            && write_file(&files.owner, &worker.pid().to_string()).is_ok()
            && fs::remove_file(&files.spawn).is_ok()
            && write_file(&ready_path(&self.store_dir, id), "").is_ok();
        if claimed {
            self.workers().insert(id.to_string(), worker);
        } else {
            drop(worker);
            self.forget(id);
        }
    }

    fn fail(&self, id: &str, message: &str) {
        let files = self.files(id);
        write_file(&files.error, message).ok();
        fs::remove_file(&files.spawn).ok();
    }
}

pub struct CursorShutdown(Arc<CursorHost>);

impl CursorShutdown {
    pub fn start(store_dir: PathBuf) -> Self {
        let host = CursorHost::new(store_dir);
        let polled = Arc::clone(&host);
        thread::spawn(move || loop {
            thread::sleep(HANDOFF_POLL);
            polled.poll();
        });
        Self(host)
    }
}

pub struct ExitCleanup {
    watches: Watches,
    cursor: Arc<CursorHost>,
}

impl ExitCleanup {
    pub fn new(watches: Watches, cursor: &CursorShutdown) -> Self {
        Self {
            watches,
            cursor: Arc::clone(&cursor.0),
        }
    }

    pub fn run(&self) {
        self.watches.kill_all();
        self.cursor.shutdown();
    }
}

impl Drop for CursorShutdown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

fn request_ids(store_dir: &Path, dir: &str) -> Vec<String> {
    fs::read_dir(store_dir.join(dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

pub fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
