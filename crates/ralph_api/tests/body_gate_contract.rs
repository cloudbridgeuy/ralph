#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::TestServer;
use std::io::Write;

fn unsent_body_head(target: &str, host: &str) -> String {
    format!(
        "POST {target} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nContent-Length: 262144\r\nConnection: close\r\n\r\n"
    )
}

fn answer_without_body(server: &TestServer, head: &str) -> u16 {
    let mut stream = server.connect();
    stream.write_all(head.as_bytes()).unwrap();
    common::read_reply(&mut stream).status
}

#[test]
fn unauthorised_render_is_401_without_the_body() {
    let server = TestServer::start();
    let head = unsent_body_head("/api/render", &server.own_host());
    assert_eq!(answer_without_body(&server, &head), 401);
}

#[test]
fn wrong_host_render_is_403_without_the_body() {
    let server = TestServer::start();
    let target = format!("/api/render?token={}", server.token());
    let head = unsent_body_head(&target, "evil.test");
    assert_eq!(answer_without_body(&server, &head), 403);
}

#[test]
fn authorised_post_to_another_path_is_answered_without_the_body() {
    let server = TestServer::start();
    let target = format!("/api/health?token={}", server.token());
    let head = unsent_body_head(&target, &server.own_host());
    assert_eq!(answer_without_body(&server, &head), 405);
}
