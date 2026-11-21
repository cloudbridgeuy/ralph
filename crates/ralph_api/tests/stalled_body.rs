#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{read_reply, Host, TestServer};
use std::io::Write;
use std::time::{Duration, Instant};

const BUDGET: Duration = Duration::from_secs(1);

#[test]
fn stalled_body_does_not_block_other_requests_or_stop() {
    let server = TestServer::start();
    let mut stalled = server.connect();
    let head = format!(
        "POST /api/health?token={} HTTP/1.1\r\nHost: {}\r\nContent-Length: 1000000\r\n\r\nabc",
        server.token(),
        server.own_host()
    );
    stalled.write_all(head.as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    let started = Instant::now();
    let target = format!("/api/health?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Own, &[]);
    let answered = started.elapsed();
    assert_eq!(reply.status, 200);
    assert!(answered < BUDGET, "second request took {answered:?}");

    let stopped = server.stop_and_time();
    assert!(stopped < BUDGET, "stop took {stopped:?}");
    drop(stalled);
}

#[test]
fn stalled_body_gets_a_rejection_without_waiting_for_the_body() {
    let server = TestServer::start();
    let mut stalled = server.connect();
    let head = format!(
        "POST /api/health?token={} HTTP/1.1\r\nHost: {}\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\nabc",
        server.token(),
        server.own_host()
    );
    stalled.write_all(head.as_bytes()).unwrap();
    let started = Instant::now();
    let reply = read_reply(&mut stalled);
    assert_eq!(reply.status, 405);
    assert!(started.elapsed() < BUDGET);
}
