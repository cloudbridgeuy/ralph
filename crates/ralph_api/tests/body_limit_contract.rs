#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, TestServer};
use std::io::Write;

const OVER_CAP: usize = 256 * 1024 + 1;

#[test]
fn declared_length_over_cap_on_a_non_render_path_is_413() {
    let server = TestServer::start();
    let head = format!(
        "POST /api/health?token={} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {OVER_CAP}\r\nConnection: close\r\n\r\nabc",
        server.token(),
        server.own_host()
    );
    assert_golden("too_large_other_path", &server.send_raw(head.as_bytes()));
}

#[test]
fn chunked_body_over_cap_on_render_is_413() {
    let server = TestServer::start();
    let head = format!(
        "POST /api/render?token={} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{OVER_CAP:x}\r\n",
        server.token(),
        server.own_host()
    );
    let mut stream = server.connect();
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(&vec![b'a'; OVER_CAP]).unwrap();
    stream.write_all(b"\r\n").unwrap();
    let reply = common::read_reply(&mut stream);
    assert_eq!(reply.status, 413);
    assert_eq!(reply.body, r#"{"error":"too large"}"#);
}
