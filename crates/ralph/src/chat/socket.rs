use crate::chat::memory::Store;
use crate::chat::suggestion::SuggestionSlot;
use crate::cursor_owner::process_start;
use crate::schedule_store::ScheduleStore;
use crate::warn::warn;
use crate::watch_requests;
use crate::workers::{SpawnArgs, Workers};
use chrono::{Local, Utc};
use ralph_core::chat_socket::{
    check_socket_path, decode_line, dir_name, dir_pid, encode_line, orphan_sidecar_fate,
    sidecar_name, sidecar_pid, socket_fate, socket_file_name, SocketFacts, SocketFate, SocketOwner,
    SocketRequest, SocketResponse,
};
use ralph_core::chat_suggestion::parse_suggestion;
use ralph_core::schedule_request::{
    duplicate_message, proposed_message, ScheduleRequest, SCHEDULE_DIR,
};
use ralph_core::watch_request::queued_message;
use ralph_core::worker::{command_allowed, parse_command, Command};
use std::collections::BTreeSet;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const SUGGEST_REPLY: &str = "suggest_reply";
const REQUEST_LIMIT_BYTES: u64 = 1 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_RETRY: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub(crate) struct SocketFiles {
    dir: PathBuf,
    socket: PathBuf,
    sidecar: PathBuf,
}

impl SocketFiles {
    fn in_dir(dir: &Path, pid: u32) -> Self {
        let private = dir.join(dir_name(pid));
        Self {
            socket: private.join(socket_file_name()),
            dir: private,
            sidecar: dir.join(sidecar_name(pid)),
        }
    }

    pub(crate) fn remove(&self) {
        fs::remove_dir_all(&self.dir).ok();
        fs::remove_file(&self.sidecar).ok();
    }
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
    pub(crate) fn bind(store_dir: &Path) -> io::Result<Self> {
        sweep_stale(store_dir);
        let pid = process::id();
        let files = SocketFiles::in_dir(store_dir, pid);
        check_socket_path(&files.socket).map_err(io::Error::other)?;
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

    pub(crate) fn serve(self, store: Store, workers: Workers, suggestions: SuggestionSlot) {
        let shared = Arc::new((store, workers, suggestions));
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

fn sweep_stale(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let dir_pids: BTreeSet<u32> = names.iter().filter_map(|name| dir_pid(name)).collect();
    let orphan_pids: BTreeSet<u32> = names
        .iter()
        .filter_map(|name| sidecar_pid(name))
        .filter(|pid| !dir_pids.contains(pid))
        .collect();
    for (pids, decide) in [
        (&dir_pids, socket_fate as fn(SocketFacts) -> SocketFate),
        (&orphan_pids, orphan_sidecar_fate),
    ] {
        for &pid in pids {
            let files = SocketFiles::in_dir(dir, pid);
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

fn handle_connection(
    stream: &UnixStream,
    shared: &(Store, Workers, SuggestionSlot),
) -> io::Result<()> {
    let (store, workers, suggestions) = shared;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(REQUEST_LIMIT_BYTES)).read_line(&mut line)?;
    let response = match decode_line::<SocketRequest>(&line) {
        Ok(request) => respond(store, workers, suggestions, &request),
        Err(text) => SocketResponse {
            text,
            is_error: true,
        },
    };
    let mut writer = stream;
    writer.write_all(encode_line(&response).as_bytes())?;
    writer.flush()
}

fn run_command(
    store: &Store,
    workers: &Workers,
    suggestions: &SuggestionSlot,
    request: &SocketRequest,
) -> Result<String, String> {
    if request.tool == SUGGEST_REPLY {
        suggestions.set(parse_suggestion(&request.args)?);
        return Ok(
            "Suggested reply set. Now write your reply; do not mention the suggestion.".to_string(),
        );
    }
    let command = parse_command(&request.tool, &request.args)
        .ok_or_else(|| format!("Unknown tool '{}'.", request.tool))??;
    let pending = store
        .pending_turn()
        .map_err(|e| format!("Could not read chat memory: {e}"))?;
    command_allowed(&command, pending)?;
    match &command {
        Command::Spawn {
            id,
            task,
            harness,
            model,
            ticket,
        } => workers.spawn(&SpawnArgs {
            id,
            task,
            harness: *harness,
            model: model.as_deref(),
            ticket: ticket.as_deref(),
        }),
        Command::Reply {
            id,
            text,
            interrupt,
        } => workers.reply(id, text, *interrupt),
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

pub(crate) fn respond(
    store: &Store,
    workers: &Workers,
    suggestions: &SuggestionSlot,
    request: &SocketRequest,
) -> SocketResponse {
    match run_command(store, workers, suggestions, request) {
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
    use ralph_core::chat_memory::Trigger;
    use serde_json::{json, Value};

    fn workers() -> Workers {
        Workers::new("/s".into(), "/bin/ralph".into(), "auto".into(), None)
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
        let response = respond(
            &store,
            &workers(),
            &SuggestionSlot::default(),
            &request("watch", watch_args()),
        );
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
        assert!(
            respond(
                &store,
                &workers(),
                &SuggestionSlot::default(),
                &request("watch", args)
            )
            .is_error
        );
        assert!(watch_requests::take_all(dir.path()).is_empty());
    }

    fn schedule_response(store: &Store, args: Value) -> SocketResponse {
        respond(
            store,
            &workers(),
            &SuggestionSlot::default(),
            &request("schedule", args),
        )
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
            &store,
            &workers(),
            &SuggestionSlot::default(),
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
            &store,
            &workers(),
            &SuggestionSlot::default(),
            &request("spawn_worker", json!({"id":"A","task":"t"})),
        );
        assert!(response.is_error);
    }

    #[test]
    fn invalid_arguments_and_unknown_tools_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let bad = respond(
            &store,
            &workers(),
            &SuggestionSlot::default(),
            &request("reply", json!({"id":"A"})),
        );
        assert!(bad.is_error);
        let unknown = respond(
            &store,
            &workers(),
            &SuggestionSlot::default(),
            &request("zoom", json!({})),
        );
        assert!(unknown.is_error);
        assert!(unknown.text.contains("Unknown tool"));
    }

    #[test]
    fn suggest_reply_stores_the_newest_text_without_a_pending_turn() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let slot = SuggestionSlot::default();
        for text in ["first", " second "] {
            let call = request("suggest_reply", json!({"text": text}));
            let response = respond(&store, &workers(), &slot, &call);
            assert!(!response.is_error);
            assert_eq!(
                response.text,
                "Suggested reply set. Now write your reply; do not mention the suggestion."
            );
        }
        assert_eq!(slot.take(), Some("second".to_string()));
    }

    #[test]
    fn invalid_suggest_reply_is_an_error_and_keeps_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let slot = SuggestionSlot::default();
        slot.set("kept".to_string());
        let call = request("suggest_reply", json!({"text": "a\nb"}));
        let response = respond(&store, &workers(), &slot, &call);
        assert!(response.is_error);
        assert!(response.text.contains("line break"));
        assert_eq!(slot.take(), Some("kept".to_string()));
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

    #[test]
    fn bound_socket_serves_a_forwarded_request_and_is_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let socket = ChatSocket::bind(dir.path()).unwrap();
        let path = socket.path().to_path_buf();
        let files = socket.files().clone();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dir_mode = fs::metadata(&files.dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(files.dir, dir.path().join(dir_name(process::id())));
        assert_eq!(path, files.dir.join("chat.sock"));
        let sidecar_mode = fs::metadata(&files.sidecar).unwrap().permissions().mode() & 0o777;
        assert_eq!(sidecar_mode, 0o600);
        let store = Store::new(dir.path().to_path_buf());
        store.set_pending_turn(Trigger::User).unwrap();
        socket.serve(
            Store::new(dir.path().to_path_buf()),
            workers(),
            SuggestionSlot::default(),
        );
        let response = forward(&path, &request("watch", watch_args())).unwrap();
        assert!(!response.is_error);
        assert_eq!(watch_requests::take_all(dir.path()).len(), 1);
        files.remove();
        assert!(!path.exists());
        assert!(!files.dir.exists());
        assert!(!files.sidecar.exists());
    }

    #[test]
    fn bind_sweeps_stale_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let own_pid = process::id();
        let dead_record = SocketFiles::in_dir(dir.path(), 2_000_001);
        let missing_record = SocketFiles::in_dir(dir.path(), 2_000_002);
        let junk_record = SocketFiles::in_dir(dir.path(), 2_000_003);
        let mismatch = SocketFiles::in_dir(dir.path(), own_pid);
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
        sweep_stale(dir.path());
        for gone in [&dead_record, &missing_record, &junk_record, &mismatch] {
            assert!(!gone.dir.exists());
            assert!(!gone.socket.exists());
            assert!(!gone.sidecar.exists());
        }
    }

    #[test]
    fn bind_keeps_a_socket_whose_owner_is_alive_with_matching_start() {
        let dir = tempfile::tempdir().unwrap();
        let parent = std::os::unix::process::parent_id();
        let Some(start) = process_start(parent) else {
            return;
        };
        let files = SocketFiles::in_dir(dir.path(), parent);
        fs::create_dir(&files.dir).unwrap();
        fs::write(&files.socket, "").unwrap();
        fs::write(&files.sidecar, SocketOwner { pid: parent, start }.render()).unwrap();
        let socket = ChatSocket::bind(dir.path()).unwrap();
        assert!(files.socket.exists());
        assert!(files.sidecar.exists());
        socket.files().remove();
    }

    #[test]
    fn bind_sweeps_orphan_owner_files_by_liveness() {
        let dir = tempfile::tempdir().unwrap();
        let own_pid = process::id();
        let Some(own_start) = process_start(own_pid) else {
            return;
        };
        let dead = SocketFiles::in_dir(dir.path(), 2_000_011);
        let live = SocketFiles::in_dir(dir.path(), own_pid);
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
        sweep_stale(dir.path());
        assert!(!dead.sidecar.exists());
        assert!(live.sidecar.exists());
    }

    #[test]
    fn bind_fails_when_the_private_directory_survives_the_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let own_pid = process::id();
        let Some(start) = process_start(own_pid) else {
            return;
        };
        let files = SocketFiles::in_dir(dir.path(), own_pid);
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
        let error = ChatSocket::bind(dir.path()).err().unwrap();
        assert!(error.to_string().contains("private chat socket directory"));
    }

    #[test]
    fn bind_fails_clearly_when_the_socket_path_is_too_long() {
        let dir = tempfile::tempdir().unwrap();
        let long = dir.path().join("d".repeat(120));
        fs::create_dir_all(&long).unwrap();
        let error = ChatSocket::bind(&long).err().unwrap();
        assert!(error.to_string().contains("--store"));
    }
}
