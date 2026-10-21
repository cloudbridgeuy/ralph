use chrono::{DateTime, Utc};
use ralph_core::wire::build::{build_call, CallInputs};
use ralph_core::wire::headers::{forward_headers, Direction};
use ralph_core::wire::provider::{Provider, RequestHead};
use ralph_core::wire::record::CallRecord;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CHUNK_SIZE: usize = 8192;
const CLIENT_DISCONNECTED: &str = "client disconnected";

#[derive(Default)]
struct Capture {
    status: Option<u16>,
    first_byte_at: Option<DateTime<Utc>>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    error: Option<String>,
}

pub struct Proxy {
    provider: Box<dyn Provider>,
    upstream: String,
    agent: ureq::Agent,
    store_events: bool,
    seq: AtomicU64,
    records: Sender<CallRecord>,
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

fn bad_gateway(client: &mut dyn Write, message: &str) -> io::Result<()> {
    let headers = [
        ("content-type".to_string(), "text/plain".to_string()),
        ("content-length".to_string(), message.len().to_string()),
    ];
    write_head(client, ureq::http::StatusCode::BAD_GATEWAY, &headers)?;
    client.write_all(message.as_bytes())?;
    client.flush()
}

fn relay_body(reader: &mut dyn Read, client: &mut dyn Write, capture: &mut Capture) {
    let mut chunk = [0u8; CHUNK_SIZE];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => {
                if client
                    .write_all(b"0\r\n\r\n")
                    .and_then(|_| client.flush())
                    .is_err()
                {
                    capture.error = Some(CLIENT_DISCONNECTED.to_string());
                }
                return;
            }
            Ok(n) => {
                capture.first_byte_at.get_or_insert_with(Utc::now);
                if write_chunk(client, &chunk[..n]).is_err() {
                    capture.error = Some(CLIENT_DISCONNECTED.to_string());
                    return;
                }
                capture.body.extend_from_slice(&chunk[..n]);
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
        records: Sender<CallRecord>,
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
        }
    }

    pub fn handle(&self, mut request: tiny_http::Request) {
        let started_at = Utc::now();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let head = request_head(&request);
        let mut body = Vec::new();
        let mut capture = Capture::default();
        match request.as_reader().read_to_end(&mut body) {
            Ok(_) => self.relay(request, &head, &body, &mut capture),
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
        let _ = self.records.send(record);
    }

    fn send(
        &self,
        head: &RequestHead,
        body: &[u8],
    ) -> Result<ureq::http::Response<ureq::Body>, String> {
        let url = format!("{}{}", self.upstream.trim_end_matches('/'), head.path);
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
        let response = match self.send(head, body) {
            Ok(response) => response,
            Err(error) => {
                let message = format!("upstream unreachable: {error}");
                let _ = bad_gateway(&mut client, &message);
                capture.error = Some(message);
                return;
            }
        };
        let status = response.status();
        capture.status = Some(status.as_u16());
        capture.headers = response_headers(&response);
        let mut forwarded = forward_headers(&raw_response_headers(&response), Direction::Response);
        forwarded.push(("transfer-encoding".to_string(), b"chunked".to_vec()));
        if write_head(&mut client, status, &forwarded).is_err() {
            capture.error = Some(CLIENT_DISCONNECTED.to_string());
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
        let headers = [("x-weird".to_string(), b"caf\xe9-\xff".to_vec())];
        write_head(&mut out, ureq::http::StatusCode::OK, &headers).unwrap();
        let needle = b"x-weird: caf\xe9-\xff\r\n";
        assert!(out.windows(needle.len()).any(|w| w == needle));
    }
}
