#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, Host, TestServer};

fn health_with(
    name: &str,
    method: &str,
    target: impl Fn(&str) -> String,
    host: Host,
    headers: impl Fn(&str) -> Vec<(String, String)>,
) {
    let server = TestServer::start();
    let token = server.token().to_string();
    let owned = headers(&token);
    let borrowed: Vec<(&str, &str)> = owned
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let reply = server.exchange(method, &target(&token), host, &borrowed);
    assert_golden(name, &reply);
}

fn none(_: &str) -> Vec<(String, String)> {
    vec![]
}

fn header(name: &str, value: String) -> Vec<(String, String)> {
    vec![(name.to_string(), value)]
}

#[test]
fn bearer_token() {
    health_with(
        "bearer_token",
        "GET",
        |_| "/api/health".into(),
        Host::Own,
        |t| header("Authorization", format!("Bearer {t}")),
    );
}

#[test]
fn query_token() {
    health_with(
        "query_token",
        "GET",
        |t| format!("/api/health?token={t}"),
        Host::Own,
        none,
    );
}

#[test]
fn cookie_token() {
    health_with(
        "cookie_token",
        "GET",
        |_| "/api/health".into(),
        Host::Own,
        |t| header("Cookie", format!("ralph_token={t}")),
    );
}

#[test]
fn header_right_query_wrong() {
    health_with(
        "header_right_query_wrong",
        "GET",
        |_| "/api/health?token=wrong".into(),
        Host::Own,
        |t| header("Authorization", format!("Bearer {t}")),
    );
}

#[test]
fn header_wrong_query_right() {
    health_with(
        "header_wrong_query_right",
        "GET",
        |t| format!("/api/health?token={t}"),
        Host::Own,
        |_| header("Authorization", "Bearer wrong".to_string()),
    );
}

#[test]
fn missing_token() {
    health_with(
        "missing_token",
        "GET",
        |_| "/api/health".into(),
        Host::Own,
        none,
    );
}

#[test]
fn wrong_token() {
    health_with(
        "wrong_token",
        "GET",
        |_| "/api/health?token=wrong".into(),
        Host::Own,
        none,
    );
}

#[test]
fn empty_token() {
    health_with(
        "empty_token",
        "GET",
        |_| "/api/health?token=".into(),
        Host::Own,
        none,
    );
}

#[test]
fn bad_host() {
    health_with(
        "bad_host",
        "GET",
        |t| format!("/api/health?token={t}"),
        Host::Named("evil.example"),
        none,
    );
}

#[test]
fn missing_host() {
    health_with(
        "missing_host",
        "GET",
        |t| format!("/api/health?token={t}"),
        Host::Absent,
        none,
    );
}

#[test]
fn localhost_host() {
    let server = TestServer::start();
    let port = server.own_host().rsplit(':').next().unwrap().to_string();
    let host = format!("localhost:{port}");
    let target = format!("/api/health?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named(&host), &[]);
    assert_golden("localhost_host", &reply);
}

#[test]
fn post_health() {
    health_with(
        "post_health",
        "POST",
        |t| format!("/api/health?token={t}"),
        Host::Own,
        none,
    );
}

#[test]
fn unknown_path_without_token() {
    health_with(
        "unknown_path_without_token",
        "GET",
        |_| "/api/nope".into(),
        Host::Own,
        none,
    );
}

#[test]
fn unknown_path_with_token() {
    health_with(
        "unknown_path_with_token",
        "GET",
        |t| format!("/api/nope?token={t}"),
        Host::Own,
        none,
    );
}

#[test]
fn query_with_extra_params() {
    health_with(
        "query_with_extra_params",
        "GET",
        |t| format!("/api/health?a=1&token={t}&b=2"),
        Host::Own,
        none,
    );
}
