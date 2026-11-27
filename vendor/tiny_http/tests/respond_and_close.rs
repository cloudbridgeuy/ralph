use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::spawn;
use std::time::Duration;
use tiny_http::{Response, Server, Timeouts};

#[test]
fn respond_and_close_sends_the_response_then_ends_the_connection_without_a_drain() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener_with_timeouts(listener, None, Timeouts::default()).unwrap();
    let handler = spawn(move || {
        let request = server.recv().unwrap();
        request
            .respond_and_close(Response::from_string("busy").with_status_code(503), || {})
            .unwrap();
    });
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .write_all(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1000000\r\n\r\n")
        .unwrap();
    let mut received = Vec::new();
    client.read_to_end(&mut received).unwrap();
    handler.join().unwrap();
    let text = String::from_utf8(received).unwrap();
    assert!(text.starts_with("HTTP/1.1 503 "), "{text}");
    assert!(text.ends_with("busy"), "{text}");
}
