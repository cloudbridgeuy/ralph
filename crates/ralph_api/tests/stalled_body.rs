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
    assert_eq!(statuses.len(), MAX_HANDLERS + 1);
    assert!(statuses.iter().all(|status| [413, 503].contains(status)));
    assert!(statuses.iter().filter(|status| **status == 413).count() >= MAX_HANDLERS);
    assert!(
        server.stop_within(BUDGET),
        "serve did not stop within {BUDGET:?}"
    );
}

fn start_limited(read: Duration) -> (u16, String, Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let store = std::env::temp_dir();
    let requested = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let limits = Limits {
        read,
        write: Duration::from_secs(10),
        header: Duration::from_secs(10),
    };
    let server = bind_with_limits(requested, &BindPolicy::loopback(), &limits).unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let config = ApiConfig {
        store,
        allowed_hosts: allowed_hosts(&["127.0.0.1", "localhost"], port),
        version: "0.0.0-test".to_string(),
        tokens_css: String::new(),
        cookie_name: cookie_name(port),
        global_config: std::env::temp_dir().join("ralph-stalled-global.toml"),
        env: |_| None,
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

const POLL_DEADLINE: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

fn poll_until_served(mut probe: impl FnMut() -> u16, deadline: Duration) -> (u16, usize) {
    let started = Instant::now();
    let mut attempts = 0;
    loop {
        attempts += 1;
        let status = probe();
        if status != 503 || started.elapsed() >= deadline {
            return (status, attempts);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn rejected_requests_release_their_slots_before_the_body_drain() {
    let server = TestServer::start();
    let (_stalled, statuses) = stall_clients(&server, MAX_HANDLERS);
    assert_eq!(statuses, vec![413; MAX_HANDLERS]);
    let target = format!("/api/health?token={}", server.token());
    let (status, _) = poll_until_served(
        || server.exchange("GET", &target, Host::Own, &[]).status,
        POLL_DEADLINE,
    );
    assert_eq!(status, 200, "slots stayed held during the body drain");
}

#[test]
fn polling_is_bounded_by_the_deadline_over_the_interval() {
    let deadline = Duration::from_millis(100);
    let (status, attempts) = poll_until_served(|| 503, deadline);
    assert_eq!(status, 503);
    let cap = (deadline.as_millis() / POLL_INTERVAL.as_millis()) as usize + 1;
    assert!(attempts <= cap, "{attempts} attempts exceed the cap {cap}");
}

#[test]
fn polling_stops_at_the_first_non_503() {
    let mut replies = vec![503, 503, 200].into_iter().rev().collect::<Vec<_>>();
    let (status, attempts) = poll_until_served(|| replies.pop().unwrap(), POLL_DEADLINE);
    assert_eq!((status, attempts), (200, 3));
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
    let (status, _) = poll_until_served(|| health_status(port, &token), POLL_DEADLINE);
    assert_eq!(status, 200, "slots were not freed before the deadline");
    stop.store(true, Ordering::SeqCst);
    thread.join().unwrap();
}
