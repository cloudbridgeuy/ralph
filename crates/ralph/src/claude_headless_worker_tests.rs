use super::*;
use ralph_core::worker::Report;
use tempfile::TempDir;

const ASK: &str = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"t1"}}"#;

#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        locked(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Run {
    shared: Shared,
    sink: Sink,
    dir: TempDir,
    events: Vec<WorkerEvent>,
}

impl Run {
    fn written(&self) -> String {
        String::from_utf8(locked(&self.sink.0).clone()).unwrap()
    }

    fn reports(&self) -> Vec<Report> {
        let Ok(entries) = fs::read_dir(self.dir.path().join("inbox")) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }

    fn permission_events(&self) -> Vec<&WorkerEvent> {
        self.events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    WorkerEvent::PermissionAsked(_) | WorkerEvent::PermissionSettled
                )
            })
            .collect()
    }
}

fn run(mode: PermissionMode, lines: &[&str]) -> Run {
    run_after(mode, |_| {}, lines)
}

fn run_after(mode: PermissionMode, setup: impl FnOnce(&Shared), lines: &[&str]) -> Run {
    let dir = TempDir::new().unwrap();
    let store = dir.path();
    let sink = Sink::default();
    let events = Events::default();
    let inbox = || Inbox::new(store.join("inbox"));
    let shared = Shared {
        id: "w".into(),
        turns: Mutex::default(),
        stdin: Mutex::new(Some(Box::new(sink.clone()))),
        events: Arc::clone(&events),
        permissions: PermissionQueue::new("p".into(), "w".into(), inbox(), Arc::clone(&events)),
        mode,
        heartbeat: Heartbeat::new(store.join("activity"), store.join("gone")),
        markers: WorkerMarkers::new(store, "w"),
        inbox: inbox(),
        closing: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        results: store.join("interrupt-result"),
        expected_session: None,
        turn_seq: AtomicU64::new(0),
        usage: None,
    };
    setup(&shared);
    let (tx, rx) = mpsc::channel();
    for line in lines {
        tx.send((*line).to_string()).unwrap();
    }
    drop(tx);
    let (ready, _) = mpsc::channel();
    shared.read(rx, Some(ready));
    let events = locked(&events).clone();
    Run {
        shared,
        sink,
        dir,
        events,
    }
}

#[test]
fn a_permission_line_without_a_request_id_posts_one_failure_and_queues_nothing() {
    let line = ASK.replace(r#""request_id":"r1","#, "");
    let run = run(PermissionMode::Ask, &[&line]);
    assert!(run.permission_events().is_empty());
    let reports = run.reports();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, "blocked");
    assert_eq!(
        reports[0].text,
        "claude sent a permission request ralph can't answer (no request_id or no input object); the worker is stuck — close it."
    );
}

#[test]
fn yolo_never_queues_or_publishes() {
    let run = run(PermissionMode::Yolo, &[ASK]);
    assert!(run.permission_events().is_empty());
    assert!(run.reports().is_empty());
}

#[test]
fn a_cancel_settles_by_the_raw_id_after_the_queue_published_the_prefixed_id() {
    let cancel = r#"{"type":"control_cancel_request","request_id":"r1"}"#;
    let run = run(PermissionMode::Ask, &[ASK, cancel]);
    let asked = run.permission_events();
    assert!(matches!(
        asked.as_slice(),
        [WorkerEvent::PermissionAsked(content), WorkerEvent::PermissionSettled]
            if content.request_id == "p.r1"
    ));
    let stale = run
        .shared
        .permissions
        .take_answer("p.r1", "allow_once")
        .err()
        .unwrap();
    assert!(stale.starts_with("stale request"));
}

#[test]
fn the_session_record_holds_the_session_id_and_spawn_settings() {
    let request = SpawnRequest {
        worktree: "/w".into(),
        exe: "/ralph".into(),
        inbox: "/store/inbox".into(),
        task: "t".into(),
        at: 1,
        snapshots: false,
        model: Some("haiku".into()),
    };
    let record = session_record("sid", &request, Path::new("/store"), PermissionMode::Yolo);
    assert_eq!(record.session_id, "sid");
    assert_eq!(record.cwd, PathBuf::from("/w"));
    assert_eq!(record.store, Some(PathBuf::from("/store")));
    assert_eq!(record.model.as_deref(), Some("haiku"));
    assert_eq!(record.permissions, Some(PermissionMode::Yolo));
}

fn spawn_request() -> SpawnRequest {
    SpawnRequest {
        worktree: "/w".into(),
        exe: "/ralph".into(),
        inbox: "/store/inbox".into(),
        task: "t".into(),
        at: 1,
        snapshots: false,
        model: None,
    }
}

fn start_files(dir: &TempDir) -> (HandoffFiles, SessionRecord) {
    let files = HandoffFiles::claude_headless(dir.path(), "w");
    let record = session_record("sid", &spawn_request(), dir.path(), PermissionMode::Ask);
    (files, record)
}

fn paths<'a>(files: &'a HandoffFiles, config: &'a Path) -> StartPaths<'a> {
    StartPaths { files, config }
}

#[test]
fn an_interrupt_denies_each_queued_permission_before_the_interrupt_line() {
    let second = ASK.replace(r#""request_id":"r1""#, r#""request_id":"r2""#);
    let run = run(PermissionMode::Ask, &[ASK, &second]);
    let lines = run.shared.interrupt_sends("turn-4");
    assert_eq!(
        lines,
        interrupt_lines(&["r1".into(), "r2".into()], "turn-4")
    );
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains(r#""behavior":"deny""#));
    assert!(lines[2].contains("interrupt-turn-4"));
    assert!(run.shared.permissions.take_answer("p.r1", "deny").is_err());
}

#[test]
fn a_new_session_writes_the_owner_and_the_record_before_turn_one() {
    let dir = TempDir::new().unwrap();
    let (files, record) = start_files(&dir);
    let config = worker_config_path(dir.path(), "w");
    let steps = start_steps(&ClaudeSession::New("sid"));
    write_start_files(
        steps,
        &paths(&files, &config),
        &record,
        std::process::id(),
        None,
    )
    .unwrap();
    assert!(files.owner.exists());
    assert_eq!(fs::read_to_string(&files.session).unwrap(), record.render());
}

#[test]
fn a_resume_writes_the_owner_and_leaves_the_record_alone() {
    let dir = TempDir::new().unwrap();
    let (files, record) = start_files(&dir);
    let config = worker_config_path(dir.path(), "w");
    let steps = start_steps(&ClaudeSession::Resume("sid"));
    write_start_files(
        steps,
        &paths(&files, &config),
        &record,
        std::process::id(),
        None,
    )
    .unwrap();
    assert!(files.owner.exists());
    assert!(!files.session.exists());
}

#[test]
fn a_failed_record_write_removes_the_owner_file_and_reports_the_error() {
    let dir = TempDir::new().unwrap();
    let (files, record) = start_files(&dir);
    let config = worker_config_path(dir.path(), "w");
    fs::write(dir.path().join(CLAUDE_HEADLESS_SESSION_DIR), "").unwrap();
    let steps = start_steps(&ClaudeSession::New("sid"));
    let error = write_start_files(
        steps,
        &paths(&files, &config),
        &record,
        std::process::id(),
        None,
    )
    .unwrap_err();
    assert!(error.starts_with("could not write the session record"));
    assert!(!files.owner.exists());
    assert!(!files.session.exists());
}

#[test]
fn a_failed_spawn_removes_only_the_worker_config() {
    let dir = TempDir::new().unwrap();
    let (files, _) = start_files(&dir);
    let config = worker_config_path(dir.path(), "w");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "{}").unwrap();
    remove_leftovers(
        cleanup(StartFault::SpawnFailed),
        &paths(&files, &config),
        None,
    );
    assert!(!config.exists());
}

#[test]
fn init_failure_and_not_adopted_remove_the_owner_the_record_and_the_config() {
    for fault in [StartFault::InitFailed, StartFault::NotAdopted] {
        let dir = TempDir::new().unwrap();
        let (files, record) = start_files(&dir);
        let config = worker_config_path(dir.path(), "w");
        let steps = start_steps(&ClaudeSession::New("sid"));
        write_start_files(
            steps,
            &paths(&files, &config),
            &record,
            std::process::id(),
            None,
        )
        .unwrap();
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "{}").unwrap();
        remove_leftovers(cleanup(fault), &paths(&files, &config), None);
        assert!(!files.owner.exists());
        assert!(!files.session.exists());
        assert!(!config.exists());
    }
}

fn ask_from(line: &str) -> ClaudePermission {
    let ClaudeLine::Permission(ask) = decode_line(line) else {
        panic!("not a permission line");
    };
    ask
}

#[test]
fn a_permission_that_arrives_after_the_interrupt_drain_is_not_queued() {
    let run = run(PermissionMode::Ask, &[]);
    run.shared.interrupt_sends("turn-4");
    run.shared.ask(ask_from(ASK));
    let stale = run
        .shared
        .permissions
        .take_answer("p.r1", "deny")
        .err()
        .unwrap();
    assert!(stale.starts_with("stale request"), "{stale}");
}

fn end_turn(run: &Run) {
    run.shared.perform(vec![Action::Emit(WorkerEvent::TurnEnded(
        ralph_core::worker::TurnEnd::Succeeded,
    ))]);
}

#[test]
fn a_refused_permission_writes_a_deny_line() {
    let run = run(PermissionMode::Ask, &[]);
    run.shared.interrupt_sends("turn-4");
    run.shared.ask(ask_from(ASK));
    assert_eq!(run.written(), format!("{}\n", deny_line("r1")));
}

#[test]
fn a_turn_end_clears_the_refusal() {
    let run = run(PermissionMode::Ask, &[]);
    run.shared.interrupt_sends("turn-4");
    end_turn(&run);
    run.shared.ask(ask_from(ASK));
    assert!(run.written().is_empty());
    assert!(run
        .shared
        .permissions
        .take_answer("p.r1", "allow_once")
        .is_ok());
}

#[test]
fn an_interrupt_after_the_turn_end_clear_keeps_the_refusal() {
    let run = run(PermissionMode::Ask, &[]);
    end_turn(&run);
    run.shared.interrupt_sends("turn-5");
    run.shared.ask(ask_from(ASK));
    assert_eq!(run.written(), format!("{}\n", deny_line("r1")));
    assert!(run.shared.permissions.take_answer("p.r1", "deny").is_err());
}

#[test]
fn an_interrupt_pending_when_claude_exits_writes_gone() {
    let run = run_after(
        PermissionMode::Ask,
        |shared| {
            shared.accept("task".into());
            shared.interrupt("stop".into(), "e1".into());
        },
        &[],
    );
    let result = fs::read_to_string(run.dir.path().join("interrupt-result").join("e1"));
    assert_eq!(result.ok().as_deref(), Some("gone"));
}

#[test]
fn an_interrupt_for_a_worker_without_a_process_writes_gone() {
    let dir = TempDir::new().unwrap();
    let host = ClaudeHeadlessHost::new(dir.path().to_path_buf());
    let files = host.files("w");
    write_file(&files.interrupt.join("e1"), "stop").unwrap();
    host.poll();
    assert!(!files.interrupt.join("e1").exists());
    assert_eq!(
        fs::read_to_string(files.interrupt_result.join("e1"))
            .ok()
            .as_deref(),
        Some("gone")
    );
}

const TRANSCRIPT_LINE: &str = r#"{"type":"assistant","message":{"id":"msg_011CggUooV9a78WEoGFHAqhN","model":"claude-haiku-5-5","stop_reason":"end_turn","usage":{"input_tokens":2,"cache_creation_input_tokens":5149,"cache_read_input_tokens":2810,"output_tokens":1371,"output_tokens_details":{"thinking_tokens":1168},"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":5149,"ephemeral_5m_input_tokens":0},"inference_geo":"global","iterations":[{"input_tokens":2,"output_tokens":1371,"cache_read_input_tokens":2810,"cache_creation_input_tokens":5149,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":5149},"type":"message"}],"speed":"standard","fallback_credit":null}}}"#;

fn with_usage(run: &mut Run) -> PathBuf {
    let store = run.dir.path();
    let transcript = store.join("s1.jsonl");
    fs::write(&transcript, TRANSCRIPT_LINE).unwrap();
    run.shared.usage = Some(HeadlessUsage {
        target: UsageTarget {
            worker: "w".into(),
            seen: store.join("w.usage-seen"),
            store: Some(store.to_path_buf()),
            usage: store.join("usage.jsonl"),
        },
        session_id: "s1".into(),
        transcript,
        finished: Mutex::default(),
    });
    store.join("usage.jsonl")
}

fn rows(usage: &Path) -> Vec<Value> {
    fs::read_to_string(usage)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_turn_end_records_the_transcript_as_one_claude_headless_row() {
    let mut run = run(PermissionMode::Ask, &[]);
    let usage = with_usage(&mut run);
    end_turn(&run);
    end_turn(&run);
    let rows = rows(&usage);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row["kind"], "worker");
    assert_eq!(row["worker_id"], "w");
    assert_eq!(row["harness"], "claude-headless");
    assert_eq!(row["model"], "claude-haiku-5-5");
    assert_eq!(row["session_id"], "s1");
    assert_eq!(row["input_tokens"], 2);
    assert_eq!(row["cache_read_input_tokens"], 2810);
    assert_eq!(row["cache_creation_input_tokens"], 5149);
    assert_eq!(row["cache_write_1h"], 5149);
    assert_eq!(row["output_tokens"], 1371);
    assert_eq!(row["reasoning_tokens"], 1168);
    assert!(row["cost_usd"].is_null());
    assert!(row["turn_id"].is_null());
    assert!(row.get("cost_source").is_none());
}

#[test]
fn claude_exit_records_the_rows_not_yet_seen() {
    let mut run = run(PermissionMode::Ask, &[]);
    let usage = with_usage(&mut run);
    let (lines, rx) = mpsc::channel::<String>();
    drop(lines);
    run.shared.read(rx, None);
    assert_eq!(rows(&usage).len(), 1);
}

#[test]
fn close_records_usage_before_it_frees_the_owner_file() {
    let mut run = run(PermissionMode::Ask, &[]);
    let usage = with_usage(&mut run);
    // The owner file doubles as the transcript: freeing the owner first
    // would leave nothing to record, so a row proves the order.
    let owner = run.shared.usage.as_ref().unwrap().transcript.clone();
    release_owner(&run.shared, &owner);
    assert!(!owner.exists());
    assert_eq!(rows(&usage).len(), 1);
}

#[test]
fn a_record_after_close_writes_no_rows_even_without_the_seen_file() {
    let mut run = run(PermissionMode::Ask, &[]);
    let usage = with_usage(&mut run);
    let owner = run.dir.path().join("owner");
    fs::write(&owner, "").unwrap();
    release_owner(&run.shared, &owner);
    fs::remove_file(run.dir.path().join("w.usage-seen")).unwrap();
    run.shared.record_usage();
    assert_eq!(rows(&usage).len(), 1);
}

#[test]
fn a_turn_end_leaves_close_free_to_record_later_messages() {
    let mut run = run(PermissionMode::Ask, &[]);
    let usage = with_usage(&mut run);
    end_turn(&run);
    let transcript = run.shared.usage.as_ref().unwrap().transcript.clone();
    let second = TRANSCRIPT_LINE.replace("msg_011CggUooV9a78WEoGFHAqhN", "msg_2");
    fs::write(&transcript, format!("{TRANSCRIPT_LINE}\n{second}")).unwrap();
    let owner = run.dir.path().join("owner");
    fs::write(&owner, "").unwrap();
    release_owner(&run.shared, &owner);
    assert_eq!(rows(&usage).len(), 2);
}
