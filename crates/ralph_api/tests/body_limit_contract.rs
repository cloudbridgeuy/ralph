#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, TestServer};

#[test]
fn render_too_large() {
    let server = TestServer::start();
    let head = format!(
        "POST /api/health?token={} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: 262145\r\nConnection: close\r\n\r\nabc",
        server.token(),
        server.own_host()
    );
    assert_golden("render_too_large", &server.send_raw(head.as_bytes()));
}
