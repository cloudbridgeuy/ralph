use crate::chat::send_it::SendIt;
use crate::chat::suggestion::SuggestionSlot;
use crate::schedule_store::ScheduleStore;
use crate::watch_requests;
use chrono::{Local, Utc};
use ralph_core::chat_memory::Trigger;
use ralph_core::chat_send_it::{parse_step_result, STEP_RESULT};
use ralph_core::chat_socket::{
    check_socket_path, decode_line, encode_line, orphan_sidecar_fate, runtime_dir_pid, sidecar_pid,
    socket_fate, SocketFacts, SocketFate, SocketOwner, SocketRequest, SocketResponse, StoreKey,
};
use ralph_core::chat_suggestion::parse_suggestion;
use ralph_core::schedule_request::{
    duplicate_message, proposed_message, ScheduleRequest, SCHEDULE_DIR,
};
use ralph_core::schedule_worker::reply_refusal;
use ralph_core::watch_request::queued_message;
use ralph_core::worker::{command_allowed, parse_command, refused_tool, Command};
use ralph_server::chat::memory::Store;
use ralph_server::chat::socket::{SocketFiles, SocketPlace};
use ralph_server::cursor_owner::process_start;
use ralph_server::warn::warn;
use ralph_server::wire::store::ensure_store_root;
use ralph_server::workers::{canonical_store, SpawnArgs, Workers};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const SUGGEST_REPLY: &str = "suggest_reply";
const PROPOSE_RUN: &str = "propose_run";
type Roster = Box<dyn Fn() -> String + Send + Sync>;
const WORKERS: &str = "workers";
const REQUEST_LIMIT_BYTES: u64 = 1 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_RETRY: Duration = Duration::from_millis(50);

pub(crate) struct Shared {
    pub(crate) store: Store,
    pub(crate) workers: Workers,
    pub(crate) suggestions: SuggestionSlot,
    pub(crate) send_it: SendIt,
}

pub(crate) struct SocketGuard(pub(crate) SocketFiles);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        self.0.remove();
    }
}

pub(crate) struct ChatSocket {
    listener: UnixListener,
    files: SocketFiles,
}

impl ChatSocket {
    pub(crate) fn bind(store_dir: &Path, root: &Path) -> io::Result<Self> {
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
        DirBuilder::new()
            .mode(0o700)
            .create(&files.dir)
            .map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!(
                        "cannot create the private chat socket directory {}: {e}",
                        files.dir.display()
                    ),
                )
            })?;
        let bound = UnixListener::bind(&files.socket).map(|listener| Self {
            listener,
            files: files.clone(),
        });
        let socket = match bound {
            Ok(socket) => socket,
            Err(e) => {
                files.remove();
                return Err(e);
            }
        };
        if let Err(e) = socket.publish(&SocketOwner { pid, start }) {
            socket.files.remove();
            return Err(e);
        }
        Ok(socket)
    }

    fn publish(&self, owner: &SocketOwner) -> io::Result<()> {
        fs::set_permissions(&self.files.socket, fs::Permissions::from_mode(0o600))?;
        let mut sidecar = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.files.sidecar)?;
        sidecar.write_all(owner.render().as_bytes())?;
        Ok(())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.files.socket
    }

    pub(crate) fn files(&self) -> &SocketFiles {
        &self.files
    }

    pub(crate) fn serve(self, shared: Shared, roster: impl Fn() -> String + Send + Sync + 'static) {
        let shared = Arc::new((shared, Box::new(roster) as Roster));
        thread::spawn(move || {
            for connection in self.listener.incoming() {
                match connection {
                    Ok(stream) => {
                        let shared = Arc::clone(&shared);
                        thread::spawn(move || {
                            if let Err(e) = handle_connection(&stream, &shared) {
                                warn(format!("chat socket: connection failed: {e}"));
                            }
                        });
                    }
                    Err(e) => {
                        warn(format!("chat socket: accept failed: {e}"));
                        thread::sleep(ACCEPT_RETRY);
                    }
                }
            }
        });
    }
}

fn entry_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn sweep_stale(place: &SocketPlace) {
    let dir_pids: BTreeSet<u32> = entry_names(place.root)
        .iter()
        .filter_map(|name| runtime_dir_pid(&place.key, name))
        .collect();
    let orphan_pids: BTreeSet<u32> = entry_names(place.store)
        .iter()
        .filter_map(|name| sidecar_pid(name))
        .filter(|pid| !dir_pids.contains(pid))
        .collect();
    for (pids, decide) in [
        (&dir_pids, socket_fate as fn(SocketFacts) -> SocketFate),
        (&orphan_pids, orphan_sidecar_fate),
    ] {
        for &pid in pids {
            let files = SocketFiles::new(place, pid);
            if decide_fate(&files, pid, decide) == SocketFate::Delete {
                files.remove();
            }
        }
    }
}

fn decide_fate(files: &SocketFiles, pid: u32, decide: fn(SocketFacts) -> SocketFate) -> SocketFate {
    let owner = fs::read_to_string(&files.sidecar)
        .ok()
        .and_then(|text| SocketOwner::parse(&text));
    let start_now = owner.as_ref().and_then(|owner| process_start(owner.pid));
    decide(SocketFacts {
        file_pid: pid,
        owner: owner.as_ref(),
        owner_start_now: start_now.as_deref(),
    })
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
        suggestions,
        send_it,
    } = shared;
    if request.tool == SUGGEST_REPLY {
        suggestions.set(parse_suggestion(&request.args)?);
        return Ok(
            "Suggested reply set. Now write your reply; do not mention the suggestion.".to_string(),
        );
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
        Command::Close { id, abandon, force } => workers.close(id, *abandon, *force),
        Command::Watch(request) => {
            watch_requests::queue(store.dir(), request).map(|()| queued_message(&request.id))
        }
        Command::Schedule(request) => propose_schedule(store, request).map_err(io::Error::other),
    }
    .map_err(|e| e.to_string())
}

fn propose_schedule(store: &Store, request: &ScheduleRequest) -> Result<String, String> {
    let schedule = ScheduleStore::new(store.dir().join(SCHEDULE_DIR));
    if schedule.load_all().iter().any(|task| task.id == request.id) {
        return Err(duplicate_message(&request.id));
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

pub(crate) fn respond(shared: &Shared, request: &SocketRequest) -> SocketResponse {
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

pub(crate) fn forward(socket: &Path, request: &SocketRequest) -> Result<SocketResponse, String> {
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
    use ralph_core::chat_send_it::Limits;
    use ralph_core::chat_socket::sidecar_name;
    use serde_json::{json, Value};
    use std::path::PathBuf;

    fn workers() -> Workers {
        Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None)
    }

    fn shared(dir: &Path) -> Shared {
        Shared {
            store: Store::new(dir.to_path_buf()),
            workers: workers(),
            suggestions: SuggestionSlot::default(),
            send_it: SendIt::new(
                dir,
                Limits {
                    max_turns: 3,
                    max_usd: 1.0,
                },
            ),
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
    fn suggest_reply_stores_the_newest_text_without_a_pending_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let shared = shared(store.dir());
        for text in ["first", " second "] {
            let call = request("suggest_reply", json!({"text": text}));
            let response = respond(&shared, &call);
            assert!(!response.is_error);
            assert_eq!(
                response.text,
                "Suggested reply set. Now write your reply; do not mention the suggestion."
            );
        }
        assert_eq!(shared.suggestions.take(), Some("second".to_string()));
    }

    #[test]
    fn invalid_suggest_reply_is_an_error_and_keeps_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let shared = shared(store.dir());
        shared.suggestions.set("kept".to_string());
        let call = request("suggest_reply", json!({"text": "a\nb"}));
        let response = respond(&shared, &call);
        assert!(response.is_error);
        assert!(response.text.contains("line break"));
        assert_eq!(shared.suggestions.take(), Some("kept".to_string()));
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
    fn bound_socket_lives_under_the_runtime_root_and_its_owner_record_in_the_store() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let files = socket.files().clone();
        let root_mode = fs::metadata(&dirs.root).unwrap().permissions().mode() & 0o777;
        assert_eq!(root_mode, 0o700);
        assert_eq!(files.dir.parent(), Some(dirs.root.as_path()));
        assert_eq!(files.sidecar, dirs.store.join(sidecar_name(process::id())));
        assert!(files.sidecar.exists());
        assert!(fs::read_dir(&dirs.store)
            .unwrap()
            .flatten()
            .all(|entry| entry.path() == files.sidecar));
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
    fn bind_sweeps_only_this_stores_runtime_dirs() {
        let dirs = dirs();
        let other = dirs.store.join("threads/other");
        fs::create_dir_all(&other).unwrap();
        let mine = files_for(&dirs, 2_000_021);
        let foreign = SocketFiles::new(&place(&other, &dirs.root), 2_000_022);
        for files in [&mine, &foreign] {
            fs::create_dir(&files.dir).unwrap();
            fs::write(&files.socket, "").unwrap();
        }
        sweep(&dirs);
        assert!(!mine.dir.exists());
        assert!(foreign.dir.exists());
    }

    #[test]
    fn bound_socket_serves_a_forwarded_request_and_is_owner_only() {
        let dirs = dirs();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        let path = socket.path().to_path_buf();
        let files = socket.files().clone();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(&files.dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(files.dir, files_for(&dirs, process::id()).dir);
        assert_eq!(path, files.dir.join("chat.sock"));
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
        assert!(!files.dir.exists());
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
            fs::create_dir(&files.dir).unwrap();
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
            assert!(!gone.dir.exists());
            assert!(!gone.socket.exists());
            assert!(!gone.sidecar.exists());
        }
    }

    #[test]
    fn bind_keeps_a_socket_whose_owner_is_alive_with_matching_start() {
        let dirs = dirs();
        let parent = std::os::unix::process::parent_id();
        let Some(start) = process_start(parent) else {
            return;
        };
        let files = files_for(&dirs, parent);
        fs::create_dir(&files.dir).unwrap();
        fs::write(&files.socket, "").unwrap();
        fs::write(&files.sidecar, SocketOwner { pid: parent, start }.render()).unwrap();
        let socket = ChatSocket::bind(&dirs.store, &dirs.root).unwrap();
        assert!(files.socket.exists());
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
        fs::create_dir(&files.dir).unwrap();
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
        assert!(error.to_string().contains("private chat socket directory"));
    }

    #[test]
    fn bind_fails_clearly_when_the_runtime_root_is_too_long() {
        let dirs = dirs();
        let root = dirs.root.join("d".repeat(100));
        let error = ChatSocket::bind(&dirs.store, &root).err().unwrap();
        assert!(error.to_string().contains(&root.display().to_string()));
    }
}
