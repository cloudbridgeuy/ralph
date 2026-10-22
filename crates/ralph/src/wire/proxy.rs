use chrono::{DateTime, Utc};
use ralph_core::wire::build::{build_call, CallInputs};
use ralph_core::wire::headers::{forward_headers, Direction};
use ralph_core::wire::provider::{Provider, RequestHead};
use ralph_core::wire::record::CallRecord;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use super::shutdown::INTERRUPTED;
use super::store::ToStore;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CHUNK_SIZE: usize = 8192;
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const CLIENT_DISCONNECTED: &str = "client disconnected";

#[derive(Default)]
struct Capture {
    status: Option<u16>,
    first_byte_at: Option<DateTime<Utc>>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    error: Option<String>,
}

struct OpenCall {
    seq: u64,
    started_at: DateTime<Utc>,
    head: RequestHead,
    request_body: Vec<u8>,
}

pub struct Proxy {
    provider: Box<dyn Provider>,
    upstream: String,
    agent: ureq::Agent,
    store_events: bool,
    seq: AtomicU64,
    records: Sender<ToStore>,
    open: Mutex<BTreeMap<u64, OpenCall>>,
    closed: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

fn request_head(request: &tiny_http::Request) -> RequestHead {
    RequestHead {
        method: request.method().as_str().to_string(),
        path: request.url().to_string(),
        headers: request
            .headers()
            .iter()
            .map(|h| (h.field.as_str().to_string(), h.value.as_str().to_string()))
            .collect(),
    }
}

fn response_headers(response: &ureq::http::Response<ureq::Body>) -> Vec<(String, String)> {
    response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

fn raw_response_headers(response: &ureq::http::Response<ureq::Body>) -> Vec<(String, Vec<u8>)> {
    response
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str().to_string(), value.as_bytes().to_vec()))
        .collect()
}

fn write_head<V: AsRef<[u8]>>(
    client: &mut dyn Write,
    status: ureq::http::StatusCode,
    headers: &[(String, V)],
) -> io::Result<()> {
    write!(
        client,
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("")
    )?;
    for (name, value) in headers {
        write!(client, "{name}: ")?;
        client.write_all(value.as_ref())?;
        client.write_all(b"\r\n")?;
    }
    client.write_all(b"Connection: close\r\n\r\n")?;
    client.flush()
}

fn write_chunk(client: &mut dyn Write, bytes: &[u8]) -> io::Result<()> {
    write!(client, "{:x}\r\n", bytes.len())?;
    client.write_all(bytes)?;
    client.write_all(b"\r\n")?;
    client.flush()
}

fn upstream_url(upstream: &str, path: &str) -> Option<String> {
    path.starts_with('/')
        .then(|| format!("{}{path}", upstream.trim_end_matches('/')))
}

fn has_body(method: &str, status: u16) -> bool {
    !(method.eq_ignore_ascii_case("HEAD") || status < 200 || status == 204 || status == 304)
}

fn append_capped(body: &mut Vec<u8>, bytes: &[u8], limit: usize) -> bool {
    let kept = limit.saturating_sub(body.len()).min(bytes.len());
    body.extend_from_slice(&bytes[..kept]);
    kept < bytes.len()
}

fn write_error(
    client: &mut dyn Write,
    status: ureq::http::StatusCode,
    message: &str,
) -> io::Result<()> {
    let headers = [
        ("content-type".to_string(), "text/plain".to_string()),
        ("content-length".to_string(), message.len().to_string()),
    ];
    write_head(client, status, &headers)?;
    client.write_all(message.as_bytes())?;
    client.flush()
}

fn relay_body(reader: &mut dyn Read, client: &mut dyn Write, capture: &mut Capture) {
    let mut chunk = [0u8; CHUNK_SIZE];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => {
                if client
                    .write_all(b"0\r\n\r\n")
                    .and_then(|_| client.flush())
                    .is_err()
                {
                    capture.error = Some(CLIENT_DISCONNECTED.to_string());
                } else if truncated {
                    capture.error =
                        Some(format!("response body truncated at {MAX_BODY_BYTES} bytes"));
                }
                return;
            }
            Ok(n) => {
                capture.first_byte_at.get_or_insert_with(Utc::now);
                if write_chunk(client, &chunk[..n]).is_err() {
                    capture.error = Some(CLIENT_DISCONNECTED.to_string());
                    return;
                }
                truncated |= append_capped(&mut capture.body, &chunk[..n], MAX_BODY_BYTES);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                capture.error = Some(format!("upstream read failed: {error}"));
                return;
            }
        }
    }
}

impl Proxy {
    pub fn new(
        provider: Box<dyn Provider>,
        upstream: String,
        store_events: bool,
        records: Sender<ToStore>,
    ) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .proxy(None)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .build()
            .into();
        Self {
            provider,
            upstream,
            agent,
            store_events,
            seq: AtomicU64::new(0),
            records,
            open: Mutex::new(BTreeMap::new()),
            closed: AtomicBool::new(false),
        }
    }

    pub fn stop_accepting(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub fn open_len(&self) -> usize {
        lock(&self.open).len()
    }

    /// Record every call still open as incomplete, then stop the writer.
    pub fn close_open(&self) {
        let open: Vec<OpenCall> = {
            let mut guard = lock(&self.open);
            std::mem::take(&mut *guard).into_values().collect()
        };
        for call in open {
            self.record(self.interrupted(call));
        }
        let _ = self.records.send(ToStore::Stop);
    }

    pub fn handle(&self, mut request: tiny_http::Request) {
        let started_at = Utc::now();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let head = request_head(&request);
        if self.closed.load(Ordering::SeqCst) {
            self.record(self.interrupted(OpenCall {
                seq,
                started_at,
                head,
                request_body: Vec::new(),
            }));
            return;
        }
        lock(&self.open).insert(
            seq,
            OpenCall {
                seq,
                started_at,
                head: head.clone(),
                request_body: Vec::new(),
            },
        );
        let mut body = Vec::new();
        let mut capture = Capture::default();
        let read = request
            .as_reader()
            .take(MAX_BODY_BYTES as u64 + 1)
            .read_to_end(&mut body);
        match read {
            Ok(_) if body.len() > MAX_BODY_BYTES => {
                body = Vec::new();
                let message = format!("request body is over the limit of {MAX_BODY_BYTES} bytes");
                let _ = write_error(
                    &mut request.into_writer(),
                    ureq::http::StatusCode::PAYLOAD_TOO_LARGE,
                    &message,
                );
                capture.error = Some(message);
            }
            Ok(_) => {
                if let Some(call) = lock(&self.open).get_mut(&seq) {
                    call.request_body.clone_from(&body);
                }
                self.relay(request, &head, &body, &mut capture);
            }
            Err(error) => capture.error = Some(format!("request body read failed: {error}")),
        }
        let record = build_call(
            self.provider.as_ref(),
            CallInputs {
                seq,
                started_at,
                first_byte_at: capture.first_byte_at,
                ended_at: Utc::now(),
                head,
                request_body: body,
                status: capture.status,
                response_headers: capture.headers,
                response_body: capture.body,
                transport_error: capture.error,
                store_events: self.store_events,
            },
        );
        let mut guard = lock(&self.open);
        if guard.remove(&seq).is_none() {
            return;
        }
        self.record(record);
    }

    fn record(&self, call: CallRecord) {
        let seq = call.seq;
        if self.records.send(ToStore::Call(Box::new(call))).is_err() {
            eprintln!("ralph wire: recorder stopped; call {seq} not recorded");
        }
    }

    fn interrupted(&self, call: OpenCall) -> CallRecord {
        build_call(
            self.provider.as_ref(),
            CallInputs {
                seq: call.seq,
                started_at: call.started_at,
                first_byte_at: None,
                ended_at: Utc::now(),
                head: call.head,
                request_body: call.request_body,
                status: None,
                response_headers: Vec::new(),
                response_body: Vec::new(),
                transport_error: Some(INTERRUPTED.to_string()),
                store_events: self.store_events,
            },
        )
    }

    fn send(
        &self,
        head: &RequestHead,
        url: String,
        body: &[u8],
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut builder = ureq::http::Request::builder()
            .method(head.method.as_str())
            .uri(url);
        for (name, value) in forward_headers(&head.headers, Direction::Request) {
            builder = builder.header(name, value);
        }
        let request = builder.body(body).map_err(|e| e.to_string())?;
        self.agent.run(request).map_err(|e| e.to_string())
    }

    fn relay(
        &self,
        request: tiny_http::Request,
        head: &RequestHead,
        body: &[u8],
        capture: &mut Capture,
    ) {
        let mut client = request.into_writer();
        let Some(url) = upstream_url(&self.upstream, &head.path) else {
            let message = "request path must start with `/`".to_string();
            let _ = write_error(&mut client, ureq::http::StatusCode::BAD_REQUEST, &message);
            capture.error = Some(message);
            return;
        };
        let response = match self.send(head, url, body) {
            Ok(response) => response,
            Err(error) => {
                let message = format!("upstream unreachable: {error}");
                let _ = write_error(&mut client, ureq::http::StatusCode::BAD_GATEWAY, &message);
                capture.error = Some(message);
                return;
            }
        };
        let status = response.status();
        capture.status = Some(status.as_u16());
        capture.headers = response_headers(&response);
        let mut forwarded = forward_headers(&raw_response_headers(&response), Direction::Response);
        let with_body = has_body(&head.method, status.as_u16());
        if with_body {
            forwarded.push(("transfer-encoding".to_string(), b"chunked".to_vec()));
        }
        if write_head(&mut client, status, &forwarded).is_err() {
            capture.error = Some(CLIENT_DISCONNECTED.to_string());
            return;
        }
        if !with_body {
            return;
        }
        let mut reader = response.into_body().into_reader();
        relay_body(&mut reader, &mut client, capture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_head_keeps_non_utf8_header_bytes() {
        let mut out = Vec::new();
        let headers = [("x-weird".to_string(), b"x\xe9-\xff".to_vec())];
        write_head(&mut out, ureq::http::StatusCode::OK, &headers).unwrap();
        let needle = b"x-weird: x\xe9-\xff\r\n";
        assert!(out.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn upstream_url_requires_a_leading_slash() {
        let up = "https://api.anthropic.com/";
        assert_eq!(
            upstream_url(up, "/v1/messages").as_deref(),
            Some("https://api.anthropic.com/v1/messages")
        );
        assert_eq!(upstream_url(up, "@evil.com/x"), None);
        assert_eq!(upstream_url(up, "http://evil.com/x"), None);
        assert_eq!(upstream_url(up, "*"), None);
    }

    #[test]
    fn has_body_is_false_for_head_and_bodyless_statuses() {
        assert!(has_body("POST", 200));
        assert!(has_body("GET", 404));
        assert!(!has_body("HEAD", 200));
        assert!(!has_body("GET", 100));
        assert!(!has_body("GET", 204));
        assert!(!has_body("GET", 304));
    }

    #[test]
    fn append_capped_keeps_up_to_the_limit() {
        let mut body = Vec::new();
        assert!(!append_capped(&mut body, b"abc", 5));
        assert!(append_capped(&mut body, b"defg", 5));
        assert_eq!(body, b"abcde");
        assert!(append_capped(&mut body, b"h", 5));
        assert_eq!(body, b"abcde");
    }

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BrokenReader;

    impl Read for BrokenReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::ConnectionReset.into())
        }
    }

    #[test]
    fn relay_body_frames_chunks_and_captures() {
        let mut out = Vec::new();
        let mut capture = Capture::default();
        relay_body(&mut io::Cursor::new(b"hello"), &mut out, &mut capture);
        assert_eq!(out, b"5\r\nhello\r\n0\r\n\r\n");
        assert_eq!(capture.body, b"hello");
        assert_eq!(capture.error, None);
        assert!(capture.first_byte_at.is_some());
    }

    #[test]
    fn relay_body_reports_client_disconnect() {
        let mut capture = Capture::default();
        relay_body(
            &mut io::Cursor::new(b"hello"),
            &mut BrokenWriter,
            &mut capture,
        );
        assert_eq!(capture.error.as_deref(), Some(CLIENT_DISCONNECTED));
    }

    #[test]
    fn relay_body_reports_upstream_read_failure() {
        let mut capture = Capture::default();
        relay_body(&mut BrokenReader, &mut Vec::new(), &mut capture);
        assert!(capture.error.unwrap().starts_with("upstream read failed"));
    }
}
