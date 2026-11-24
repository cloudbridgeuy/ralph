#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]

#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{read_reply, Host, TestServer};
use ralph_api::{
    allowed_hosts, bind_with_limits, cookie_name, serve, ApiConfig, BindPolicy, Limits, Token,
    MAX_HANDLERS,
};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    assert_eq!(read_status(&mut stalled), 413);

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
    assert_eq!(reply.status, 413);
    assert!(started.elapsed() < BUDGET);
}

fn read_status(stream: &mut TcpStream) -> u16 {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let text = String::from_utf8(head).unwrap();
    text.split(' ').nth(1).unwrap().parse().unwrap()
}

fn stall_clients(server: &TestServer, count: usize) -> (Vec<TcpStream>, Vec<u16>) {
    let mut clients: Vec<TcpStream> = (0..count)
        .map(|_| {
            let mut client = server.connect();
            let head = format!(
                "POST /api/health?token={} HTTP/1.1\r\nHost: {}\r\nContent-Length: 1000000\r\nConnection: close\r\n\r\nabc",
                server.token(),
                server.own_host()
            );
            client.write_all(head.as_bytes()).unwrap();
            client
        })
        .collect();
    let statuses = clients.iter_mut().map(read_status).collect();
    (clients, statuses)
}

#[test]
fn stop_returns_when_every_slot_and_one_more_client_stall() {
    let mut server = TestServer::start();
    let (_stalled, mut statuses) = stall_clients(&server, MAX_HANDLERS + 1);
    statuses.sort_unstable();
    let mut expected = vec![413; MAX_HANDLERS];
    expected.push(503);
    assert_eq!(statuses, expected);
    assert!(
        server.stop_within(BUDGET),
        "serve did not stop within {BUDGET:?}"
    );
}

#[test]
fn over_cap_request_gets_503_busy_while_slots_are_stalled() {
    let server = TestServer::start();
    let (_stalled, statuses) = stall_clients(&server, MAX_HANDLERS);
    assert_eq!(statuses, vec![413; MAX_HANDLERS]);
    let started = Instant::now();
    let target = format!("/api/health?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Own, &[]);
    assert_eq!(reply.status, 503);
    assert_eq!(reply.body, r#"{"error":"busy"}"#);
    assert!(started.elapsed() < BUDGET);
}

fn start_limited(read: Duration) -> (u16, String, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let store = std::env::temp_dir();
    let requested = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let limits = Limits {
        read,
        write: Duration::from_secs(10),
    };
    let server = bind_with_limits(requested, &BindPolicy::loopback(), &limits).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let config = ApiConfig {
        store,
        allowed_hosts: allowed_hosts(&["127.0.0.1", "localhost"], port),
        version: "0.0.0-test".to_string(),
        tokens_css: String::new(),
        cookie_name: cookie_name(port),
    };
    let token = Token::generate().unwrap();
    let secret = token.as_str().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        serve(server, &config, &token, &|| flag.load(Ordering::SeqCst)).unwrap();
    });
    (port, secret, stop, thread)
}

fn health_status(port: u16, token: &str) -> u16 {
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!(
        "GET /api/health?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    client.write_all(request.as_bytes()).unwrap();
    read_status(&mut client)
}

#[test]
fn stalled_clients_are_closed_by_the_read_timeout_and_free_their_slots() {
    let (port, token, stop, thread) = start_limited(Duration::from_millis(200));
    let clients: Vec<_> = (0..8)
        .map(|_| {
            let token = token.clone();
            std::thread::spawn(move || {
                let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
                client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let head = format!(
                    "POST /api/health?token={token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1000000\r\n\r\n0123456789"
                );
                client.write_all(head.as_bytes()).unwrap();
                let mut sink = Vec::new();
                client.read_to_end(&mut sink)
            })
        })
        .collect();
    for client in clients {
        assert!(
            client.join().unwrap().is_ok(),
            "server did not close a stalled client"
        );
    }
    let mut status = 503;
    for _ in 0..100_000 {
        status = health_status(port, &token);
        if status != 503 {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(status, 200);
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
}
