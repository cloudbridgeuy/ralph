#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::TestServer;

const OVER_CAP: usize = 256 * 1024 + 1;
const AT_CAP: usize = 256 * 1024;

fn post(path_and_query: &str, host: &str, length: usize) -> String {
    format!(
        "POST {path_and_query} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\nabc"
    )
}

#[test]
fn over_cap_length_without_token_is_413() {
    let server = TestServer::start();
    let head = post("/api/health", &server.own_host(), OVER_CAP);
    let reply = server.send_raw(head.as_bytes());
    assert_eq!(reply.status, 413);
    assert_eq!(reply.body, r#"{"error":"too large"}"#);
}

#[test]
fn over_cap_length_on_a_bad_host_is_413() {
    let server = TestServer::start();
    let head = post(
        &format!("/api/health?token={}", server.token()),
        "evil.test",
        OVER_CAP,
    );
    assert_eq!(server.send_raw(head.as_bytes()).status, 413);
}

#[test]
fn over_cap_length_on_an_unknown_path_is_413() {
    let server = TestServer::start();
    let head = post("/nope", &server.own_host(), OVER_CAP);
    assert_eq!(server.send_raw(head.as_bytes()).status, 413);
}

#[test]
fn over_cap_length_on_an_authorised_render_is_413() {
    let server = TestServer::start();
    let head = post(
        &format!("/api/render?token={}", server.token()),
        &server.own_host(),
        OVER_CAP,
    );
    assert_eq!(server.send_raw(head.as_bytes()).status, 413);
}

#[test]
fn at_cap_length_without_token_is_401() {
    let server = TestServer::start();
    let head = post("/api/health", &server.own_host(), AT_CAP);
    assert_eq!(server.send_raw(head.as_bytes()).status, 401);
}

#[test]
fn get_without_body_and_without_token_is_still_401() {
    let server = TestServer::start();
    let head = format!(
        "GET /api/health HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        server.own_host()
    );
    assert_eq!(server.send_raw(head.as_bytes()).status, 401);
}
