#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{read_reply, Host, Reply, TestServer};
use ralph_api::MAX_HANDLERS;
use std::io::Write;
use std::net::TcpStream;
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

fn stall_client(server: &TestServer) -> (TcpStream, Reply) {
    let mut client = server.connect();
    let head = format!(
        "POST /api/health?token={} HTTP/1.1\r\nHost: {}\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\nabc",
        server.token(),
        server.own_host()
    );
    client.write_all(head.as_bytes()).unwrap();
    let reply = read_reply(&mut client);
    (client, reply)
}

fn stall_clients(server: &TestServer, count: usize) -> Vec<TcpStream> {
    (0..count)
        .map(|index| {
            let (client, reply) = stall_client(server);
            assert_eq!(reply.status, 405, "stalled client {index} held no slot");
            client
        })
        .collect()
}

#[test]
fn stop_returns_when_every_slot_and_one_more_client_stall() {
    let mut server = TestServer::start();
    let _stalled = stall_clients(&server, MAX_HANDLERS);
    let (_extra, reply) = stall_client(&server);
    assert_eq!(reply.status, 503);
    assert!(
        server.stop_within(BUDGET),
        "serve did not stop within {BUDGET:?}"
    );
}

#[test]
fn over_cap_request_gets_503_busy_while_slots_are_stalled() {
    let server = TestServer::start();
    let _stalled = stall_clients(&server, MAX_HANDLERS);
    let started = Instant::now();
    let target = format!("/api/health?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Own, &[]);
    assert_eq!(reply.status, 503);
    assert_eq!(reply.body, r#"{"error":"busy"}"#);
    assert!(started.elapsed() < BUDGET);
}
