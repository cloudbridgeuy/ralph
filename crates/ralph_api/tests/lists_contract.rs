#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use chrono::{DateTime, Utc};
use common::{assert_golden, Host, Reply, TestServer};
use ralph_core::schedule::{Task, TaskKind, TaskStatus};
use ralph_core::schedule_repeat::Repeat;
use ralph_core::schedule_request::SCHEDULE_DIR;
use ralph_server::schedule_store::ScheduleStore;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

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
fn schedules_skip_a_corrupt_file() {
    let server = TestServer::start_with_store(|dir| {
        save_all(dir, &tasks()[..1]);
        fs::write(dir.join(SCHEDULE_DIR).join("broken.json"), "{not json").unwrap();
    });
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let rows = json_of(&reply)["rows"].clone();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["id"], "nightly");
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
