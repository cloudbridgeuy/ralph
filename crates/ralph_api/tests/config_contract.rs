#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{Host, Reply, TestServer};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const FIXTURE: &str = include_str!("../fixtures/config.toml");

fn write_config(dir: &Path, text: &str) {
    fs::write(dir.join("config.toml"), text).unwrap();
}

fn get(server: &TestServer) -> Reply {
    let target = format!("/api/config?token={}", server.token());
    server.exchange("GET", &target, Host::Own, &[])
}

fn json_of(reply: &Reply) -> Value {
    serde_json::from_slice(&reply.body_bytes).unwrap()
}

#[test]
fn config_200_shape_and_redaction() {
    let server = TestServer::start_with_store(|dir| write_config(dir, FIXTURE));
    let reply = get(&server);
    assert_eq!(reply.status, 200, "{}", reply.body);
    let value = json_of(&reply);
    let name = server.store_path().file_name().unwrap().to_str().unwrap();
    assert_eq!(value["store"], name);
    let rows = value["rows"].as_array().unwrap();
    let theme = rows
        .iter()
        .find(|row| row["section"] == "theme" && row["key"] == "name")
        .unwrap();
    assert_eq!(theme["value"], "tokyo-night");
    assert_eq!(theme["redacted"], false);
    for row in rows {
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in ["section", "key", "value", "source", "redacted"] {
            assert!(keys.contains(&key), "{key}");
        }
    }
    for leak in ["abc123", "/Users", server.store_path().to_str().unwrap()] {
        assert!(!reply.body.contains(leak), "{leak}");
    }
}

#[test]
fn config_global_rows_come_from_the_injected_global_path() {
    let server =
        TestServer::start_with_dirs(|_| {}, |dir| write_config(dir, "[chat]\nwrap_width = 70\n"));
    let value = json_of(&get(&server));
    let wrap = value["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["section"] == "chat" && row["key"] == "wrap_width")
        .unwrap();
    assert_eq!(wrap["source"], "global");
    assert_eq!(wrap["value"], 70);
}

#[test]
fn config_invalid_is_422_without_paths() {
    let server = TestServer::start_with_store(|dir| write_config(dir, "[chat\nwrap_width = 1\n"));
    let reply = get(&server);
    assert_eq!(reply.status, 422, "{}", reply.body);
    assert_eq!(json_of(&reply)["error"], "config invalid");
    assert!(!reply.body.contains(server.store_path().to_str().unwrap()));
}

#[test]
fn config_unreadable_is_500() {
    let server =
        TestServer::start_with_store(|dir| fs::create_dir(dir.join("config.toml")).unwrap());
    let reply = get(&server);
    assert_eq!(reply.status, 500, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "config unavailable"}));
}

#[test]
fn config_missing_store_is_404() {
    let server = TestServer::start();
    fs::remove_dir_all(server.store_path()).unwrap();
    let reply = get(&server);
    assert_eq!(reply.status, 404, "{}", reply.body);
    assert_eq!(json_of(&reply), json!({"error": "store not found"}));
}

#[test]
fn config_post_is_405_with_allow() {
    let server = TestServer::start();
    let target = format!("/api/config?token={}", server.token());
    let reply = server.exchange("POST", &target, Host::Own, &[]);
    assert_eq!(reply.status, 405);
    assert!(reply
        .headers
        .iter()
        .any(|(name, value)| name == "Allow" && value == "GET"));
}

#[test]
fn config_without_token_is_401() {
    let server = TestServer::start();
    let reply = server.exchange("GET", "/api/config", Host::Own, &[]);
    assert_eq!(reply.status, 401);
    assert_eq!(json_of(&reply), json!({"error": "unauthorized"}));
}

#[test]
fn config_wrong_host_is_403() {
    let server = TestServer::start();
    let target = format!("/api/config?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named("evil.test"), &[]);
    assert_eq!(reply.status, 403);
}
