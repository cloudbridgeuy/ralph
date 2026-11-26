#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use chrono::{DateTime, Utc};
use common::{assert_golden, Host, Reply, TestServer};
use ralph_core::chat_memory::{encode_entry, NewEntry, Role};
use ralph_core::schedule::{Task, TaskKind, TaskStatus};
use ralph_core::schedule_repeat::Repeat;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_core::watch::{WatchKind, WatchSpec};
use ralph_core::watch_store::{SavedWatch, WATCHES_DIR};
use ralph_core::worker::{
    activity_path, blocked_path, encode_spawn_record, gone_path, spawn_record_path,
    unreported_path, waiting_path, SpawnRecord, AGY_OWNER_DIR, CLAUDE_OWNER_DIR, CODEX_OWNER_DIR,
    CURSOR_OWNER_DIR, OPENCODE_OWNER_DIR,
};
use ralph_server::schedule_store::ScheduleStore;
use ralph_server::watch_store::WatchStore;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::Duration;

fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn task(id: &str, kind: TaskKind, at: &str, status: TaskStatus) -> Task {
    Task {
        id: id.to_string(),
        kind,
        follow_up: Some("follow-up: read /Users/alice/notes and tell me".to_string()),
        at: utc(at),
        timeout_secs: None,
        status,
        late: false,
        deadline: None,
        repeat: None,
    }
}

fn command(text: &str) -> TaskKind {
    TaskKind::Command {
        command: text.to_string(),
    }
}

fn tasks() -> Vec<Task> {
    let mut nightly = task(
        "nightly",
        command("/Users/alice/secret/run.sh --token sk-live-0123456789abcdef"),
        "2026-03-01T02:00:00Z",
        TaskStatus::Pending,
    );
    nightly.repeat = Some(Repeat::Daily {
        daily: "02:00".to_string(),
    });
    let mut review = task(
        "review",
        TaskKind::Worker {
            brief: "brief: curl -H 'Authorization: Bearer ghp_TOKEN' https://x.test".to_string(),
        },
        "2026-03-01T01:00:00Z",
        TaskStatus::Running,
    );
    review.late = true;
    review.timeout_secs = Some(7200);
    review.deadline = Some(utc("2026-03-01T03:00:00Z"));
    let probe = task(
        "probe",
        command("curl -H 'Authorization: Bearer ghp_TOKEN' https://x.test"),
        "2026-03-02T09:30:00Z",
        TaskStatus::Proposed,
    );
    vec![nightly, review, probe]
}

fn save_all(store: &Path, tasks: &[Task]) {
    let schedule = ScheduleStore::new(store.join(SCHEDULE_DIR));
    for task in tasks {
        schedule.save(task).unwrap();
    }
}

fn get(server: &TestServer) -> Reply {
    let target = format!("/api/schedules?token={}", server.token());
    server.exchange("GET", &target, Host::Own, &[])
}

fn json_of(reply: &Reply) -> Value {
    serde_json::from_slice(&reply.body_bytes).unwrap()
}

fn store_name(server: &TestServer) -> String {
    server
        .store_path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

#[test]
fn schedules_200_shape_and_order() {
    let server = TestServer::start_with_store(|dir| save_all(dir, &tasks()));
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let value = json_of(&reply);
    assert_eq!(value["store"], store_name(&server));
    let rows = value["rows"].as_array().unwrap();
    let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["review", "nightly", "probe"]);
    assert_eq!(
        rows[0],
        json!({
            "id": "review", "kind": "worker", "status": "running",
            "at": "2026-03-01T01:00:00Z", "late": true,
            "deadline": "2026-03-01T03:00:00Z", "timeout_secs": 7200,
            "repeat": null, "has_follow_up": true,
        })
    );
    assert_eq!(rows[1]["repeat"], "daily 02:00");
    assert_eq!(rows[2]["status"], "proposed");
}

#[test]
fn schedules_missing_dir_is_empty_rows() {
    let server = TestServer::start();
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(json_of(&reply)["rows"], json!([]));
}

#[test]
fn schedules_skip_a_corrupt_file_and_keep_it_on_disk() {
    let server = TestServer::start_with_store(|dir| {
        save_all(dir, &tasks()[..1]);
        fs::write(dir.join(SCHEDULE_DIR).join("broken.json"), "{not json").unwrap();
    });
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let rows = json_of(&reply)["rows"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "nightly");
    assert!(!reply.body.contains("broken"), "{}", reply.body);
    assert!(
        server
            .store_path()
            .join(SCHEDULE_DIR)
            .join("broken.json")
            .exists(),
        "a GET must not delete a schedule file"
    );
}

#[test]
fn schedules_missing_store_is_404() {
    let server = TestServer::start();
    fs::remove_dir_all(server.store_path()).unwrap();
    let reply = get(&server);
    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "store not found"}));
}

#[test]
fn schedules_post_is_405_with_allow() {
    let server = TestServer::start();
    let target = format!("/api/schedules?token={}", server.token());
    let reply = server.exchange("POST", &target, Host::Own, &[]);
    assert_eq!(reply.status, 405);
    assert!(reply
        .headers
        .iter()
        .any(|(name, value)| name == "Allow" && value == "GET"));
}

#[test]
fn schedules_without_token_is_401() {
    let server = TestServer::start();
    let reply = server.exchange("GET", "/api/schedules", Host::Own, &[]);
    assert_eq!(reply.status, 401);
    assert_eq!(json_of(&reply), json!({"error": "unauthorized"}));
}

#[test]
fn schedules_wrong_host_is_403() {
    let server = TestServer::start();
    let target = format!("/api/schedules?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named("evil.test"), &[]);
    assert_eq!(reply.status, 403);
}

#[test]
fn schedules_body_matches_the_golden_and_leaks_nothing() {
    let server = TestServer::start_with_store(|dir| save_all(dir, &tasks()));
    let reply = get(&server);
    let name = store_name(&server);
    let leaks = [
        "/Users",
        "sk-live",
        "Bearer",
        "curl",
        "ghp_TOKEN",
        "follow-up",
        "brief",
        server.store_path().to_str().unwrap(),
    ];
    for leak in leaks {
        assert!(!reply.body.contains(leak), "{leak}");
    }
    assert_golden(
        "lists/schedules",
        &reply.redacting(&[(name.as_str(), "store")]),
    );
}

fn watch(
    id: &str,
    command: &str,
    kind: WatchKind,
    timeout: Option<u64>,
    start: &str,
) -> SavedWatch {
    SavedWatch::new(
        &WatchSpec {
            id: id.to_string(),
            command: command.to_string(),
            follow_up: Some("follow-up: read /Users/alice/notes and tell me".to_string()),
            kind,
            timeout: timeout.map(Duration::from_secs),
            limit: timeout.map(Duration::from_secs),
            cwd: None,
        },
        utc(start),
    )
}

fn watches() -> Vec<SavedWatch> {
    let mut quiet = watch(
        "w2",
        "sleep 600",
        WatchKind::Persistent,
        None,
        "2026-03-01T09:05:00Z",
    );
    quiet.follow_up = None;
    vec![
        quiet,
        watch(
            "w1",
            "curl -H 'Authorization: Bearer ghp_TOKEN' https://x.test /Users/alice/secret/run.sh sk-live-0123456789abcdef",
            WatchKind::Normal,
            Some(3600),
            "2026-03-01T09:00:00Z",
        ),
    ]
}

fn save_watches(store: &Path, watches: &[SavedWatch]) {
    let saved = WatchStore::new(store.join(WATCHES_DIR));
    for watch in watches {
        saved.save(watch).unwrap();
    }
}

fn get_watches(server: &TestServer) -> Reply {
    let target = format!("/api/watches?token={}", server.token());
    server.exchange("GET", &target, Host::Own, &[])
}

#[test]
fn watches_200_shape_and_order() {
    let server = TestServer::start_with_store(|dir| save_watches(dir, &watches()));
    let reply = get_watches(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let value = json_of(&reply);
    assert_eq!(value["store"], store_name(&server));
    let rows = value["rows"].as_array().unwrap();
    let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["w1", "w2"]);
    assert_eq!(
        rows[0],
        json!({
            "id": "w1", "kind": "normal", "started_at": "2026-03-01T09:00:00Z",
            "timeout_secs": 3600, "timeout_deadline": "2026-03-01T10:00:00Z",
            "wake_deadline": null, "has_follow_up": true,
        })
    );
    assert_eq!(rows[1]["kind"], "persistent");
    assert_eq!(rows[1]["wake_deadline"], "2026-03-01T09:15:00Z");
    assert_eq!(rows[1]["has_follow_up"], false);
}

#[test]
fn watches_missing_dir_is_empty_rows() {
    let server = TestServer::start();
    let reply = get_watches(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(json_of(&reply)["rows"], json!([]));
}

#[test]
fn watches_skip_a_corrupt_file_and_keep_it_on_disk() {
    let server = TestServer::start_with_store(|dir| {
        save_watches(dir, &watches()[..1]);
        fs::write(dir.join(WATCHES_DIR).join("broken.json"), "{not json").unwrap();
    });
    let reply = get_watches(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let rows = json_of(&reply)["rows"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "w2");
    assert!(!reply.body.contains("broken"), "{}", reply.body);
    assert!(
        server
            .store_path()
            .join(WATCHES_DIR)
            .join("broken.json")
            .exists(),
        "a GET must not delete a watch file"
    );
}

#[test]
fn watches_missing_store_is_404() {
    let server = TestServer::start();
    fs::remove_dir_all(server.store_path()).unwrap();
    let reply = get_watches(&server);
    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "store not found"}));
}

#[test]
fn watches_post_is_405_with_allow() {
    let server = TestServer::start();
    let target = format!("/api/watches?token={}", server.token());
    let reply = server.exchange("POST", &target, Host::Own, &[]);
    assert_eq!(reply.status, 405);
    assert!(reply
        .headers
        .iter()
        .any(|(name, value)| name == "Allow" && value == "GET"));
}

#[test]
fn watches_without_token_is_401() {
    let server = TestServer::start();
    let reply = server.exchange("GET", "/api/watches", Host::Own, &[]);
    assert_eq!(reply.status, 401);
    assert_eq!(json_of(&reply), json!({"error": "unauthorized"}));
}

#[test]
fn watches_wrong_host_is_403() {
    let server = TestServer::start();
    let target = format!("/api/watches?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named("evil.test"), &[]);
    assert_eq!(reply.status, 403);
}

#[test]
fn watches_body_matches_the_golden_and_leaks_nothing() {
    let server = TestServer::start_with_store(|dir| save_watches(dir, &watches()));
    let reply = get_watches(&server);
    let name = store_name(&server);
    let leaks = [
        "/Users",
        "sk-live",
        "Bearer",
        "curl",
        "ghp_TOKEN",
        "follow-up",
        server.store_path().to_str().unwrap(),
    ];
    for leak in leaks {
        assert!(!reply.body.contains(leak), "{leak}");
    }
    assert_golden(
        "lists/watches",
        &reply.redacting(&[(name.as_str(), "store")]),
    );
}

const LEAKY: &str =
    "curl -H 'Authorization: Bearer ghp_TOKEN' /Users/alice/secret/run.sh sk-live-0123456789abcdef";

fn touch(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn owner(store: &Path, dir: &str, id: &str) {
    touch(&store.join(dir).join(id), "");
}

fn worker_store(store: &Path) {
    owner(store, CLAUDE_OWNER_DIR, "alpha");
    owner(store, CURSOR_OWNER_DIR, "bravo");
    owner(store, AGY_OWNER_DIR, "charlie");
    owner(store, CODEX_OWNER_DIR, "delta");
    owner(store, OPENCODE_OWNER_DIR, "echo");
    owner(store, CLAUDE_OWNER_DIR, "foxtrot");
    let blocked = json!({
        "request_id": "r1", "pending": ["r1", "r2"], "tool": "Bash", "summary": LEAKY,
    });
    touch(&blocked_path(store, "alpha"), &blocked.to_string());
    touch(&waiting_path(store, "bravo"), LEAKY);
    touch(&activity_path(store, "charlie"), "5 failed");
    touch(&gone_path(store, "delta"), LEAKY);
    touch(&unreported_path(store, "echo"), LEAKY);
    let record = SpawnRecord {
        ticket: Some("GUZ-1".to_string()),
        repo: Some("/Users/alice/repo".to_string()),
        lane: Some("/Users/alice/lane".to_string()),
        base_sha: Some("0123456789abcdef".to_string()),
        ..SpawnRecord::default()
    };
    touch(
        &spawn_record_path(store, "alpha"),
        &encode_spawn_record(&record).unwrap(),
    );
    let report = encode_entry(&NewEntry {
        role: Role::Worker,
        at: "2026-03-01T09:00:00Z".to_string(),
        text: format!("[worker alpha · done]\nfinished {LEAKY}"),
    })
    .unwrap();
    touch(&store.join("log.jsonl"), &report);
}

fn get_workers(server: &TestServer) -> Reply {
    let target = format!("/api/workers?token={}", server.token());
    server.exchange("GET", &target, Host::Own, &[])
}

#[test]
fn workers_200_shape_state_and_order() {
    let server = TestServer::start_with_store(worker_store);
    let reply = get_workers(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let value = json_of(&reply);
    assert_eq!(value["store"], store_name(&server));
    let rows = value["rows"].as_array().unwrap();
    let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]
    );
    assert_eq!(
        rows[0],
        json!({
            "id": "alpha", "harness": "claude", "state": "blocked", "tone": "warn",
            "pending_count": 2, "ticket": "GUZ-1",
            "last_report_status": "done", "last_report_at": "2026-03-01T09:00:00Z",
        })
    );
    let states: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| {
            (
                row["state"].as_str().unwrap(),
                row["tone"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            ("blocked", "warn"),
            ("waiting", "info"),
            ("failed", "bad"),
            ("gone", "neutral"),
            ("unreported", "accent"),
            ("running", "ok"),
        ]
    );
    assert_eq!(rows[5]["ticket"], Value::Null);
}

#[test]
fn workers_empty_dirs_are_empty_rows() {
    let server = TestServer::start_with_store(|dir| {
        for owner_dir in [CLAUDE_OWNER_DIR, CURSOR_OWNER_DIR, AGY_OWNER_DIR] {
            fs::create_dir_all(dir.join(owner_dir)).unwrap();
        }
    });
    let reply = get_workers(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(json_of(&reply)["rows"], json!([]));
}

#[test]
fn workers_missing_store_is_404() {
    let server = TestServer::start();
    fs::remove_dir_all(server.store_path()).unwrap();
    let reply = get_workers(&server);
    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "store not found"}));
}

#[test]
fn workers_post_is_405_with_allow() {
    let server = TestServer::start();
    let target = format!("/api/workers?token={}", server.token());
    let reply = server.exchange("POST", &target, Host::Own, &[]);
    assert_eq!(reply.status, 405);
    assert!(reply
        .headers
        .iter()
        .any(|(name, value)| name == "Allow" && value == "GET"));
}

#[test]
fn workers_without_token_is_401() {
    let server = TestServer::start();
    let reply = server.exchange("GET", "/api/workers", Host::Own, &[]);
    assert_eq!(reply.status, 401);
    assert_eq!(json_of(&reply), json!({"error": "unauthorized"}));
}

#[test]
fn workers_wrong_host_is_403() {
    let server = TestServer::start();
    let target = format!("/api/workers?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named("evil.test"), &[]);
    assert_eq!(reply.status, 403);
}

#[test]
fn workers_body_matches_the_golden_and_leaks_nothing() {
    let server = TestServer::start_with_store(worker_store);
    let reply = get_workers(&server);
    let name = store_name(&server);
    let leaks = [
        "/Users",
        "sk-live",
        "Bearer",
        "curl",
        "ghp_TOKEN",
        "finished",
        "0123456789abcdef",
        "Bash",
        server.store_path().to_str().unwrap(),
    ];
    for leak in leaks {
        assert!(!reply.body.contains(leak), "{leak}");
    }
    assert_golden(
        "lists/workers",
        &reply.redacting(&[(name.as_str(), "store")]),
    );
}

#[test]
fn workers_ticket_without_a_ticket_shape_is_null() {
    let server = TestServer::start_with_store(|store| {
        for (id, ticket) in [("secret", "sk-live-0123456789abcdef"), ("real", "GUZ-1748")] {
            owner(store, CLAUDE_OWNER_DIR, id);
            let record = SpawnRecord {
                ticket: Some(ticket.to_string()),
                ..SpawnRecord::default()
            };
            touch(
                &spawn_record_path(store, id),
                &encode_spawn_record(&record).unwrap(),
            );
        }
    });
    let reply = get_workers(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let rows = json_of(&reply)["rows"].clone();
    let ticket_of = |id: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|row| row["id"] == id)
            .unwrap()["ticket"]
            .clone()
    };
    assert_eq!(ticket_of("secret"), Value::Null);
    assert_eq!(ticket_of("real"), "GUZ-1748");
    assert!(!reply.body.contains("sk-live"), "{}", reply.body);
}

const PLANTED: &str = "sk-live-0123456789abcdef";

#[test]
fn schedules_repeat_with_free_text_is_not_returned() {
    let server = TestServer::start_with_store(|dir| {
        let mut planted = task(
            "planted",
            command("true"),
            "2026-03-01T02:00:00Z",
            TaskStatus::Pending,
        );
        planted.repeat = Some(Repeat::Daily {
            daily: PLANTED.to_string(),
        });
        save_all(dir, &[planted]);
    });
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(!reply.body.contains("sk-live"), "{}", reply.body);
    assert_eq!(json_of(&reply)["rows"][0]["repeat"], Value::Null);
}

#[test]
fn workers_planted_ticket_and_report_status_are_not_returned() {
    // The harness is a fixed label chosen from OWNER_DIRS in worker_facts, never
    // read from a file, so it cannot carry free text. The state, tone and
    // pending_count are fixed words or a number.
    let server = TestServer::start_with_store(|store| {
        owner(store, CLAUDE_OWNER_DIR, "alpha");
        let record = SpawnRecord {
            ticket: Some(PLANTED.to_string()),
            ..SpawnRecord::default()
        };
        touch(
            &spawn_record_path(store, "alpha"),
            &encode_spawn_record(&record).unwrap(),
        );
        let report = encode_entry(&NewEntry {
            role: Role::Worker,
            at: "2026-03-01T09:00:00Z".to_string(),
            text: format!("[worker alpha · {PLANTED}]\nbody"),
        })
        .unwrap();
        touch(&store.join("log.jsonl"), &report);
    });
    let reply = get_workers(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(!reply.body.contains("sk-live"), "{}", reply.body);
    assert!(!reply.body.contains("body"), "{}", reply.body);
    let row = json_of(&reply)["rows"][0].clone();
    assert_eq!(row["last_report_status"], Value::Null);
}

#[test]
fn workers_planted_report_time_is_not_returned() {
    let server = TestServer::start_with_store(|store| {
        owner(store, CLAUDE_OWNER_DIR, "alpha");
        let report = encode_entry(&NewEntry {
            role: Role::Worker,
            at: PLANTED.to_string(),
            text: "[worker alpha · done]\nbody".to_string(),
        })
        .unwrap();
        touch(&store.join("log.jsonl"), &report);
    });
    let reply = get_workers(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(!reply.body.contains("sk-live"), "{}", reply.body);
    let row = json_of(&reply)["rows"][0].clone();
    assert_eq!(row["last_report_status"], "done");
    assert_eq!(row["last_report_at"], Value::Null);
}
