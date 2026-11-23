#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, Reply, TestServer};

fn post(server: &TestServer, query: &str, host: &str, headers: &str, body: &[u8]) -> Reply {
    let head = format!(
        "POST /api/render{query} HTTP/1.1\r\nHost: {host}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    server.send_raw(&bytes)
}

fn authed(server: &TestServer, headers: &str, body: &[u8]) -> Reply {
    post(
        server,
        &format!("?token={}", server.token()),
        &server.own_host(),
        headers,
        body,
    )
}

const JSON: &str = "Content-Type: application/json\r\n";

#[test]
fn render_ok() {
    let server = TestServer::start();
    let reply = authed(&server, JSON, br##"{"markdown":"# Hi\n\n**bold** text"}"##);
    assert_golden("render_ok", &reply);
}

#[test]
fn render_ok_charset_any_case() {
    let server = TestServer::start();
    let reply = authed(
        &server,
        "Content-Type: Application/JSON; charset=UTF-8\r\n",
        br#"{"markdown":"x"}"#,
    );
    assert_eq!(reply.status, 200);
}

#[test]
fn render_ok_hostile() {
    let server = TestServer::start();
    let reply = authed(
        &server,
        JSON,
        br#"{"markdown":"<script>alert(1)</script> [a](javascript:x)"}"#,
    );
    assert_golden("render_ok_hostile", &reply);
}

#[test]
fn render_401() {
    let server = TestServer::start();
    let reply = post(
        &server,
        "",
        &server.own_host(),
        JSON,
        br#"{"markdown":"x"}"#,
    );
    assert_golden("render_401", &reply);
}

#[test]
fn render_403() {
    let server = TestServer::start();
    let reply = post(
        &server,
        &format!("?token={}", server.token()),
        "evil.example",
        JSON,
        br#"{"markdown":"x"}"#,
    );
    assert_golden("render_403", &reply);
}

#[test]
fn render_405() {
    let server = TestServer::start();
    let reply = server.exchange(
        "GET",
        &format!("/api/render?token={}", server.token()),
        common::Host::Own,
        &[],
    );
    assert_golden("render_405", &reply);
}

#[test]
fn render_400() {
    let server = TestServer::start();
    for (name, body) in [
        ("render_400", &b"not json"[..]),
        ("render_400_missing", br#"{"other":"x"}"#),
        ("render_400_non_string", br#"{"markdown":5}"#),
    ] {
        assert_golden(name, &authed(&server, JSON, body));
    }
    assert_eq!(
        authed(&server, JSON, b"{\"markdown\":\"\xff\"}").status,
        400
    );
    assert_eq!(authed(&server, JSON, b"").status, 400);
}

#[test]
fn render_415() {
    let server = TestServer::start();
    let reply = authed(
        &server,
        "Content-Type: text/plain\r\n",
        br#"{"markdown":"x"}"#,
    );
    assert_golden("render_415", &reply);
    assert_eq!(authed(&server, "", br#"{"markdown":"x"}"#).status, 415);
}

#[test]
fn render_413() {
    let server = TestServer::start();
    let head = format!(
        "POST /api/render?token={} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: 262145\r\nConnection: close\r\n\r\nabc",
        server.token(),
        server.own_host()
    );
    assert_golden("render_413", &server.send_raw(head.as_bytes()));
}
