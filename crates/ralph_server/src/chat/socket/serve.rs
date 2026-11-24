use crate::chat::memory::Store;
use crate::chat::send_it::SendIt;
use crate::chat::socket::sweep::sweep_stale;
use crate::chat::socket::{SocketFiles, SocketPlace};
use crate::chat::stage;
use crate::cursor_owner::process_start;
use crate::schedule_store::ScheduleStore;
use crate::warn::{warn, warn_if_err};
use crate::watch_requests;
use crate::watches::Watches;
use crate::wire::store::ensure_store_root;
use crate::worker_mcp::Inbox;
use crate::workers::{canonical_store, SpawnArgs, Workers};
use chrono::{Local, Utc};
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::{parse_step_result, STEP_RESULT};
use ralph_core::chat_socket::{
    check_socket_path, decode_line, encode_line, SocketOwner, SocketRequest, SocketResponse,
    StoreKey, MAX_FRAME_BYTES,
};
use ralph_core::schedule_repeat::repeating_cap_refusal;
use ralph_core::schedule_request::{
    duplicate_message, proposed_message, ScheduleRequest, SCHEDULE_DIR,
};
use ralph_core::schedule_worker::reply_refusal;
use ralph_core::watch_request::queued_message;
use ralph_core::worker::{command_allowed, parse_command, refused_tool, Command};
use serde_json::Value;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const PROPOSE_RUN: &str = "propose_run";
type Roster = Box<dyn Fn() -> String + Send + Sync>;
const WORKERS: &str = "workers";
const WATCH_LIST: &str = "watch_list";
const REQUEST_LIMIT_BYTES: u64 = MAX_FRAME_BYTES as u64;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_RETRY: Duration = Duration::from_millis(50);

pub struct Shared {
    pub store: Store,
    pub workers: Workers,
    pub send_it: SendIt,
    pub watches: Watches,
    pub inbox: Arc<Inbox>,
}

pub struct SocketGuard(pub SocketFiles);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        self.0.remove();
    }
}

pub struct ChatSocket {
    listener: UnixListener,
    root: PathBuf,
    files: SocketFiles,
    owner: SocketOwner,
}

#[derive(Clone)]
pub struct SocketHandle {
    inner: Arc<Mutex<ServedSocket>>,
}

struct ServedSocket {
    root: PathBuf,
    files: SocketFiles,
    owner: SocketOwner,
    shared: Arc<(Shared, Roster)>,
    stop: Arc<AtomicBool>,
}

impl SocketHandle {
    pub fn socket_path(&self) -> PathBuf {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .files
            .socket
            .clone()
    }

    pub fn rebind(&self) -> io::Result<()> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.files.socket.exists() {
            return Ok(());
        }
        ensure_store_root(&state.root)?;
        check_socket_path(&state.root, &state.files.socket).map_err(io::Error::other)?;
        let listener = bind_listener(&state.files.socket)?;
        write_sidecar(&state.files.sidecar, &state.owner)?;
        state.stop.store(true, Ordering::SeqCst);
        let next_stop = Arc::new(AtomicBool::new(false));
        spawn_accept_loop(listener, Arc::clone(&state.shared), Arc::clone(&next_stop));
        state.stop = next_stop;
        Ok(())
    }
}

fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn write_sidecar(path: &Path, owner: &SocketOwner) -> io::Result<()> {
    let mut sidecar = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    sidecar.write_all(owner.render().as_bytes())
}

fn spawn_accept_loop(listener: UnixListener, shared: Arc<(Shared, Roster)>, stop: Arc<AtomicBool>) {
    thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(&stream, &shared) {
                        warn(format!("chat socket: connection failed: {e}"));
                    }
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_RETRY);
            }
            Err(e) => {
                warn(format!("chat socket: accept failed: {e}"));
                thread::sleep(ACCEPT_RETRY);
            }
        }
    });
}

impl ChatSocket {
    pub fn bind(store_dir: &Path, root: &Path) -> io::Result<Self> {
        ensure_store_root(root)?;
        let place = SocketPlace {
            store: store_dir,
            root,
            key: StoreKey::of(&canonical_store(store_dir.to_path_buf())),
        };
        sweep_stale(&place);
        let pid = process::id();
        let files = SocketFiles::new(&place, pid);
        check_socket_path(root, &files.socket).map_err(io::Error::other)?;
        let start = process_start(pid)
            .ok_or_else(|| io::Error::other("cannot read the start time of this process"))?;
        let owner = SocketOwner { pid, start };
        DirBuilder::new()
            .mode(0o700)
            .create(&files.files_dir)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "cannot create the private chat files directory {}: {e}",
                        files.files_dir.display()
                    ),
                )
            })?;
        let listener = match bind_listener(&files.socket) {
            Ok(listener) => listener,
            Err(e) => {
                files.remove();
                return Err(e);
            }
        };
        if let Err(e) = write_sidecar(&files.sidecar, &owner) {
            files.remove();
            return Err(e);
        }
        Ok(Self {
            listener,
            root: root.to_path_buf(),
            files,
            owner,
        })
    }

    pub fn path(&self) -> &Path {
        &self.files.socket
    }

    pub fn files_dir(&self) -> &Path {
        &self.files.files_dir
    }

    pub fn files(&self) -> &SocketFiles {
        &self.files
    }

    pub fn serve(
        self,
        shared: Shared,
        roster: impl Fn() -> String + Send + Sync + 'static,
    ) -> SocketHandle {
        let shared = Arc::new((shared, Box::new(roster) as Roster));
        let stop = Arc::new(AtomicBool::new(false));
        spawn_accept_loop(self.listener, Arc::clone(&shared), Arc::clone(&stop));
        SocketHandle {
            inner: Arc::new(Mutex::new(ServedSocket {
                root: self.root,
                files: self.files,
                owner: self.owner,
                shared,
                stop,
            })),
        }
    }
}

fn handle_connection(stream: &UnixStream, (shared, roster): &(Shared, Roster)) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(REQUEST_LIMIT_BYTES)).read_line(&mut line)?;
    let response = match decode_line::<SocketRequest>(&line) {
        Ok(request) if request.tool == WORKERS => SocketResponse {
            text: roster(),
            is_error: false,
        },
        Ok(request) if request.tool == PROPOSE_RUN => {
            to_response(shared.send_it.propose(&request.args))
        }
        Ok(request) => respond(shared, &request),
        Err(text) => SocketResponse {
            text,
            is_error: true,
        },
    };
    let mut writer = stream;
    writer.write_all(encode_line(&response).as_bytes())?;
    writer.flush()
}

fn run_command(shared: &Shared, request: &SocketRequest) -> Result<String, String> {
    let Shared {
        store,
        workers,
        send_it,
        watches,
        inbox,
    } = shared;
    if request.tool == WATCH_LIST {
        return Ok(watch_requests::list_text(watches, store.dir(), Utc::now()));
    }
    if let Some(result) = stage::call(store, request) {
        return result;
    }
    let pending = store
        .pending_turn()
        .map_err(|e| format!("Could not read chat memory: {e}"))?;
    if request.tool == STEP_RESULT {
        return record_step_result(send_it, pending, &request.args);
    }
    let command = parse_command(&request.tool, &request.args)
        .ok_or_else(|| format!("Unknown tool '{}'.", request.tool))??;
    if let Err(refused) = command_allowed(&command, pending, send_it.allowed().as_deref()) {
        if refused.stops_run {
            send_it.refuse(refused_tool(&command));
        }
        return Err(refused.text);
    }
    match &command {
        Command::Spawn {
            id,
            task,
            harness,
            model,
            ticket,
            phase,
        } => workers.spawn(&SpawnArgs {
            id,
            task,
            harness: *harness,
            model: model.as_deref(),
            ticket: ticket.as_deref(),
            scheduled: false,
            phase: *phase,
        }),
        Command::Reply {
            id,
            text,
            interrupt,
            request,
        } => {
            let schedule = ScheduleStore::new(store.dir().join(SCHEDULE_DIR));
            if let Some(refusal) = reply_refusal(id, &schedule.load_all()) {
                return Err(refusal);
            }
            workers
                .reply(id, text, *interrupt, request.as_deref())
                .inspect(|_| send_it.answer(id.clone()))
        }
        Command::Close { id, abandon, force } => {
            workers.close(id, *abandon, *force).inspect(|_| {
                if *abandon {
                    warn_if_err(inbox.drop_worker(id), format!("dropping reports of {id}"));
                }
            })
        }
        Command::Watch(request) => {
            watch_requests::queue(store.dir(), request).map(|()| queued_message(&request.id))
        }
        Command::Schedule(request) => propose_schedule(store, request).map_err(io::Error::other),
        Command::WatchStop(target) => Ok(watch_requests::stop_text(
            watches,
            store.dir(),
            target,
            Utc::now(),
        )),
    }
    .map_err(|e| e.to_string())
}

fn propose_schedule(store: &Store, request: &ScheduleRequest) -> Result<String, String> {
    let schedule = ScheduleStore::new(store.dir().join(SCHEDULE_DIR));
    if schedule.load_all().iter().any(|task| task.id == request.id) {
        return Err(duplicate_message(&request.id));
    }
    if request.repeat.is_some() {
        if let Some(refusal) = repeating_cap_refusal(&schedule.load_all()) {
            return Err(refusal);
        }
    }
    let task = request.clone().into_task(Utc::now(), &Local)?;
    schedule
        .save(&task)
        .map_err(|e| format!("Could not save the schedule: {e}"))?;
    Ok(proposed_message(&task.id))
}

fn record_step_result(
    send_it: &SendIt,
    pending: Option<Trigger>,
    args: &Value,
) -> Result<String, String> {
    if pending != Some(Trigger::SendIt) {
        return Err("step_result is only for a /send-it judge turn.".to_string());
    }
    send_it.record(parse_step_result(args)?);
    Ok("Step result recorded. Reply in one short line; ralph decides the next step.".to_string())
}

pub fn respond(shared: &Shared, request: &SocketRequest) -> SocketResponse {
    to_response(run_command(shared, request))
}

fn to_response(result: Result<String, String>) -> SocketResponse {
    match result {
        Ok(text) => SocketResponse {
            text,
            is_error: false,
        },
        Err(text) => SocketResponse {
            text,
            is_error: true,
        },
    }
}

pub fn forward(socket: &Path, request: &SocketRequest) -> Result<SocketResponse, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| {
        format!(
            "no running ralph chat: cannot reach {} ({e}). Start `ralph chat` and try again.",
            socket.display()
        )
    })?;
    let lost = |e: io::Error| {
        format!(
            "lost the connection to ralph chat at {}: {e}",
            socket.display()
        )
    };
    stream
        .write_all(encode_line(request).as_bytes())
        .map_err(lost)?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(lost)?;
    if line.is_empty() {
        return Err(format!(
            "ralph chat at {} closed the connection without answering",
            socket.display()
        ));
    }
    decode_line(&line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_send_it::{Limits, VerifyMode};
    use ralph_core::chat_socket::{sidecar_name, socket_file_name};
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn workers() -> Workers {
        Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None)
    }

    fn shared(dir: &Path) -> Shared {
        let inbox = Arc::new(Inbox::new(dir.join("inbox")));
        Shared {
            store: Store::new(dir.to_path_buf()),
            workers: workers(),
            send_it: SendIt::new(
                dir,
                Limits {
                    max_turns: 3,
                    max_usd: 1.0,
                    verify: VerifyMode::Off,
                },
                Arc::new(|_| {}),
            )
            .unwrap(),
            watches: Watches::new(Arc::clone(&inbox), dir, Duration::from_secs(60)),
            inbox,
        }
    }

    fn request(tool: &str, args: Value) -> SocketRequest {
        SocketRequest {
            tool: tool.to_string(),
            args,
        }
    }

    fn watch_args() -> Value {
        json!({"id":"ci","command":"gh run watch 1","follow_up":"report","persistent":true,"timeout_secs":60})
    }

    #[test]
    fn watch_call_queues_one_request_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        let response = respond(&shared(store.dir()), &request("watch", watch_args()));
        assert!(!response.is_error);
        let queued = watch_requests::take_all(dir.path());
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].command, "gh run watch 1");
        assert!(queued[0].persistent);
        assert_eq!(queued[0].timeout_secs, Some(60));
    }

    #[test]
    fn watch_list_answers_on_any_turn_and_shows_a_queued_watch() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::Worker).unwrap();
        let shared = shared(store.dir());
        let empty = respond(&shared, &request("watch_list", json!({})));
        assert_eq!(
            (empty.text.as_str(), empty.is_error),
            ("No active watches.", false)
        );
        respond(&shared, &request("watch", watch_args()));
        let none = watch_requests::list_queued(dir.path());
        assert!(none.is_empty(), "a worker turn must not queue");
        store.set_pending_turn(Trigger::User).unwrap();
        respond(&shared, &request("watch", watch_args()));
        let listed = respond(&shared, &request("watch_list", json!({})));
        assert!(
            listed.text.starts_with("watch:ci queued persistent"),
            "{}",
            listed.text
        );
    }

    #[test]
    fn watch_stop_drops_a_queued_watch_only_on_a_user_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let shared = shared(store.dir());
        store.set_pending_turn(Trigger::User).unwrap();
        assert!(!respond(&shared, &request("watch", watch_args())).is_error);
        let stop = json!({"ids": ["ci"]});
        for refused in [Trigger::Worker, Trigger::SendIt] {
            store.set_pending_turn(refused).unwrap();
            assert!(respond(&shared, &request("watch_stop", stop.clone())).is_error);
            assert_eq!(watch_requests::list_queued(dir.path()).len(), 1);
        }
        store.set_pending_turn(Trigger::User).unwrap();
        let done = respond(&shared, &request("watch_stop", stop));
        assert!(!done.is_error, "{}", done.text);
        assert_eq!(
            done.text,
            "Removed queued watch watch:ci before it started."
        );
        assert!(watch_requests::list_queued(dir.path()).is_empty());
        let bad = respond(&shared, &request("watch_stop", json!({})));
        assert!(bad.is_error);
    }

    #[test]
    fn watch_stop_is_refused_when_no_turn_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let shared = shared(store.dir());
        store.set_pending_turn(Trigger::User).unwrap();
        assert!(!respond(&shared, &request("watch", watch_args())).is_error);
        store.clear_pending_turn();
        let refused = respond(&shared, &request("watch_stop", json!({"ids": ["ci"]})));
        assert!(refused.is_error);
        assert_eq!(watch_requests::list_queued(dir.path()).len(), 1);
    }

    #[test]
    fn watch_is_refused_on_a_worker_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::Worker).unwrap();
        let args = json!({"id":"ci","command":"c","follow_up":"f"});
        assert!(respond(&shared(store.dir()), &request("watch", args)).is_error);
        assert!(watch_requests::take_all(dir.path()).is_empty());
    }

    fn schedule_response(store: &Store, args: Value) -> SocketResponse {
        respond(&shared(store.dir()), &request("schedule", args))
    }

    fn schedule_args() -> Value {
        json!({"id":"nightly","command":"make test","after":"10m"})
    }

    #[test]
    fn schedule_call_writes_one_proposed_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        let response = schedule_response(&store, schedule_args());
        assert!(!response.is_error, "{}", response.text);
        assert!(response.text.contains("proposed"));
        let saved = ScheduleStore::new(dir.path().join(SCHEDULE_DIR)).load_all();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, "nightly");
        assert_eq!(saved[0].status, ralph_core::schedule::TaskStatus::Proposed);
    }

    #[test]
    fn repeating_schedules_are_capped_at_propose_time() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        let repeating = |id: &str| json!({"id": id, "command": "ls", "every": "30m"});
        for n in 0..4 {
            let id = format!("r{n}");
            assert!(!schedule_response(&store, repeating(&id)).is_error);
        }
        assert!(!schedule_response(&store, repeating("r4")).is_error);
        let sixth = schedule_response(&store, repeating("r5"));
        assert!(sixth.is_error);
        assert!(sixth.text.contains("At most"));
        assert!(!schedule_response(&store, schedule_args()).is_error);
    }

    #[test]
    fn schedule_rejects_a_duplicate_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        assert!(!schedule_response(&store, schedule_args()).is_error);
        let second = schedule_response(&store, schedule_args());
        assert!(second.is_error);
        assert!(second.text.contains("already exists"));
        assert_eq!(
            ScheduleStore::new(dir.path().join(SCHEDULE_DIR))
                .load_all()
                .len(),
            1
        );
    }

    #[test]
    fn schedule_is_refused_on_a_worker_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::Worker).unwrap();
        assert!(schedule_response(&store, schedule_args()).is_error);
        assert!(ScheduleStore::new(dir.path().join(SCHEDULE_DIR))
            .load_all()
            .is_empty());
    }

    #[test]
    fn schedule_with_an_unreadable_time_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        let args = json!({"id":"a","command":"c","at":"later"});
        assert!(schedule_response(&store, args).is_error);
    }

    #[test]
    fn spawn_worker_is_refused_on_a_worker_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        fs::write(
            dir.path().join("log.jsonl"),
            "{\"role\":\"worker\",\"at\":\"t\",\"text\":\"merge?\"}\n",
        )
        .unwrap();
        store.set_pending_turn(Trigger::Worker).unwrap();
        let response = respond(
            &shared(store.dir()),
            &request("spawn_worker", json!({"id":"A","task":"merge"})),
        );
        assert!(response.is_error);
        assert!(response.text.contains("Only the person"));
    }

    #[test]
    fn spawn_worker_is_refused_when_no_turn_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let response = respond(
            &shared(store.dir()),
            &request("spawn_worker", json!({"id":"A","task":"t"})),
        );
        assert!(response.is_error);
    }

    #[test]
    fn invalid_arguments_and_unknown_tools_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let bad = respond(&shared(store.dir()), &request("reply", json!({"id":"A"})));
        assert!(bad.is_error);
        let unknown = respond(&shared(store.dir()), &request("zoom", json!({})));
        assert!(unknown.is_error);
        assert!(unknown.text.contains("Unknown tool"));
    }

    #[test]
    fn forward_without_a_listener_reports_no_running_chat() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("chat-1.sock");
        let error = forward(&socket, &request("watch", watch_args())).unwrap_err();
        assert!(error.contains("no running ralph chat"));
        assert!(error.contains(&socket.display().to_string()));
    }

    #[test]
    fn forward_to_a_stale_socket_file_reports_no_running_chat() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("chat-1.sock");
        fs::write(&socket, "").unwrap();
        let error = forward(&socket, &request("watch", watch_args())).unwrap_err();
        assert!(error.contains("no running ralph chat"));
    }

    struct Dirs {
        _tmp: tempfile::TempDir,
        store: PathBuf,
        root: PathBuf,
    }

    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir_in("/tmp").unwrap();
        let store = tmp.path().join("store");
        fs::create_dir(&store).unwrap();
        let root = tmp.path().join("run");
        Dirs {
            _tmp: tmp,
            store,
            root,
        }
    }

    fn place<'a>(store: &'a Path, root: &'a Path) -> SocketPlace<'a> {
        SocketPlace {
            store,
            root,
            key: StoreKey::of(&canonical_store(store.to_path_buf())),
        }
    }

    fn files_for(dirs: &Dirs, pid: u32) -> SocketFiles {
        DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&dirs.root)
            .unwrap();
        SocketFiles::new(&place(&dirs.store, &dirs.root), pid)
    }

    fn sweep(dirs: &Dirs) {
        sweep_stale(&place(&dirs.store, &dirs.root));
    }

    #[test]
    fn bound_socket_lives_flat_in_runtime_root_and_chat_files_in_store() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let files = socket.files().clone();
        let root_mode = fs::metadata(&dirs.root).unwrap().permissions().mode() & 0o777;
        assert_eq!(root_mode, 0o700);
        assert_eq!(files.socket.parent(), Some(dirs.root.as_path()));
        assert_eq!(
            files.files_dir,
            dirs.store.join(format!("chat-{}", process::id()))
        );
        assert_eq!(files.sidecar, dirs.store.join(sidecar_name(process::id())));
        assert!(files.files_dir.is_dir());
        assert!(files.sidecar.exists());
        assert!(fs::read_dir(&dirs.store)
            .unwrap()
            .flatten()
            .all(|entry| entry.path() == files.sidecar || entry.path() == files.files_dir));
        files.remove();
    }

    #[test]
    fn bind_refuses_a_runtime_root_open_to_others() {
        let dirs = dirs();
        DirBuilder::new().mode(0o755).create(&dirs.root).unwrap();
        fs::set_permissions(&dirs.root, fs::Permissions::from_mode(0o755)).unwrap();
        let error = ChatSocket::bind(&dirs.store, &dirs.root).err().unwrap();
        assert!(error.to_string().contains("0700"), "{error}");
    }

    #[test]
    fn bind_works_for_a_store_too_deep_for_the_old_layout() {
        let dirs = dirs();
        let deep = dirs.store.join("d".repeat(100)).join("threads/viewer");
        fs::create_dir_all(&deep).unwrap();
        let socket = ChatSocket::bind(&deep, &dirs.root).unwrap();
        assert!(socket.path().starts_with(&dirs.root));
        socket.files().remove();
    }

    #[test]
    fn bind_sweeps_other_stores_dead_sockets_but_not_their_files_dirs() {
        let dirs = dirs();
        let other = dirs.store.join("threads/other");
        fs::create_dir_all(&other).unwrap();
        let mine = files_for(&dirs, 2_000_021);
        let foreign = SocketFiles::new(&place(&other, &dirs.root), 2_000_022);
        for files in [&mine, &foreign] {
            fs::create_dir(&files.files_dir).unwrap();
            fs::write(&files.socket, "").unwrap();
        }
        sweep(&dirs);
        assert!(!mine.socket.exists());
        assert!(!mine.files_dir.exists());
        assert!(!foreign.socket.exists());
        assert!(foreign.files_dir.exists());
    }

    #[test]
    fn bound_socket_serves_a_forwarded_request_and_is_owner_only() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let path = socket.path().to_path_buf();
        let files = socket.files().clone();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(&files.files_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(files.socket, files_for(&dirs, process::id()).socket);
        let sidecar_mode = fs::metadata(&files.sidecar).unwrap().permissions().mode() & 0o777;
        assert_eq!(sidecar_mode, 0o600);
        let store = Store::new(dirs.store.clone());
        store.set_pending_turn(Trigger::User).unwrap();
        socket.serve(shared(&dirs.store), String::new);
        let response = forward(&path, &request("watch", watch_args())).unwrap();
        assert!(!response.is_error);
        assert_eq!(watch_requests::take_all(&dirs.store).len(), 1);
        files.remove();
        assert!(!path.exists());
        assert!(!files.files_dir.exists());
        assert!(!files.sidecar.exists());
    }

    #[test]
    fn bound_socket_serves_workers_from_the_roster_without_a_pending_turn() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let path = socket.path().to_path_buf();
        let files = socket.files().clone();
        socket.serve(shared(&dirs.store), || "stub roster".to_string());
        let response = forward(&path, &request("workers", json!({}))).unwrap();
        assert!(!response.is_error);
        assert_eq!(response.text, "stub roster");
        files.remove();
    }

    #[test]
    fn bound_socket_stores_a_proposed_run_without_a_pending_turn() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let path = socket.path().to_path_buf();
        let files = socket.files().clone();
        socket.serve(shared(&dirs.store), String::new);
        let plan = json!({"goal": "g", "steps": [{"do": "x", "criterion": {"kind": "judge", "question": "done?"}}]});
        let response = forward(&path, &request("propose_run", plan)).unwrap();
        assert!(!response.is_error, "{}", response.text);
        assert!(dirs.store.join("send-it/run.toml").exists());
        files.remove();
    }

    #[test]
    fn bind_sweeps_stale_sockets() {
        let dirs = dirs();
        let own_pid = process::id();
        let dead_record = files_for(&dirs, 2_000_001);
        let missing_record = files_for(&dirs, 2_000_002);
        let junk_record = files_for(&dirs, 2_000_003);
        let mismatch = files_for(&dirs, own_pid);
        for files in [&dead_record, &missing_record, &junk_record, &mismatch] {
            fs::create_dir(&files.files_dir).unwrap();
            fs::write(&files.socket, "").unwrap();
        }
        let write_owner = |files: &SocketFiles, pid, start: &str| {
            let owner = SocketOwner {
                pid,
                start: start.to_string(),
            };
            fs::write(&files.sidecar, owner.render()).unwrap();
        };
        write_owner(&dead_record, 2_000_001, "never");
        write_owner(&mismatch, own_pid, "not my start");
        fs::write(&junk_record.sidecar, "junk").unwrap();
        sweep(&dirs);
        for gone in [&dead_record, &missing_record, &junk_record, &mismatch] {
            assert!(!gone.files_dir.exists());
            assert!(!gone.socket.exists());
            assert!(!gone.sidecar.exists());
        }
    }

    #[test]
    fn bind_removes_dead_sockets_of_other_stores_and_keeps_live_ones() {
        let dirs = dirs();
        let parent = std::os::unix::process::parent_id();
        let other = StoreKey::of(Path::new("/gone/store"));
        fs::create_dir_all(&dirs.root).unwrap();
        let dead = dirs.root.join(socket_file_name(&other, 2_000_021));
        let live = dirs.root.join(socket_file_name(&other, parent));
        fs::write(&dead, "").unwrap();
        fs::write(&live, "").unwrap();
        sweep(&dirs);
        assert!(!dead.exists());
        assert_eq!(live.exists(), process_start(parent).is_some());
    }

    #[test]
    fn bind_keeps_a_socket_whose_owner_is_alive_with_matching_start() {
        let dirs = dirs();
        let parent = std::os::unix::process::parent_id();
        let Some(start) = process_start(parent) else {
            return;
        };
        let files = files_for(&dirs, parent);
        fs::create_dir(&files.files_dir).unwrap();
        fs::write(&files.socket, "").unwrap();
        fs::write(&files.sidecar, SocketOwner { pid: parent, start }.render()).unwrap();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        assert!(files.socket.exists());
        assert!(files.files_dir.exists());
        assert!(files.sidecar.exists());
        socket.files().remove();
    }

    #[test]
    fn bind_sweeps_orphan_owner_files_by_liveness() {
        let dirs = dirs();
        let own_pid = process::id();
        let Some(own_start) = process_start(own_pid) else {
            return;
        };
        let dead = files_for(&dirs, 2_000_011);
        let live = files_for(&dirs, own_pid);
        let dead_owner = SocketOwner {
            pid: 2_000_011,
            start: "never".to_string(),
        };
        let live_owner = SocketOwner {
            pid: own_pid,
            start: own_start,
        };
        fs::write(&dead.sidecar, dead_owner.render()).unwrap();
        fs::write(&live.sidecar, live_owner.render()).unwrap();
        sweep(&dirs);
        assert!(!dead.sidecar.exists());
        assert!(live.sidecar.exists());
    }

    #[test]
    fn bind_fails_when_the_private_directory_survives_the_sweep() {
        let dirs = dirs();
        let own_pid = process::id();
        let Some(start) = process_start(own_pid) else {
            return;
        };
        let files = files_for(&dirs, own_pid);
        fs::create_dir(&files.files_dir).unwrap();
        fs::write(
            &files.sidecar,
            SocketOwner {
                pid: own_pid,
                start,
            }
            .render(),
        )
        .unwrap();
        let error = ChatSocket::bind(&dirs.store, &dirs.root).err().unwrap();
        assert!(error.to_string().contains("private chat files directory"));
    }

    #[test]
    fn bind_fails_clearly_when_the_runtime_root_is_too_long() {
        let dirs = dirs();
        let root = dirs.root.join("d".repeat(100));
        let error = ChatSocket::bind(&dirs.store, &root).err().unwrap();
        assert!(error.to_string().contains(&root.display().to_string()));
    }

    #[test]
    fn rebind_restores_deleted_socket_and_serves_requests() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let path = socket.path().to_path_buf();
        let files = socket.files().clone();
        let handle = socket.serve(shared(&dirs.store), || "live roster".to_string());
        fs::remove_file(&path).unwrap();
        assert!(forward(&path, &request("workers", json!({}))).is_err());
        handle.rebind().unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let response = forward(&path, &request("workers", json!({}))).unwrap();
        assert!(!response.is_error);
        assert_eq!(response.text, "live roster");
        files.remove();
    }
}
