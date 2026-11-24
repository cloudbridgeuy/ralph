#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
use ralph_api::{allowed_hosts, bind, cookie_name, serve, ApiConfig, BindPolicy, Token};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const VERSION: &str = "0.0.0-test";
pub const TOKENS_CSS: &str = ":root {\n  --r-font: x;\n}\n";
const STABLE_HEADERS: [&str; 8] = [
    "Content-Type",
    "Cache-Control",
    "Allow",
    "Location",
    "Set-Cookie",
    "Content-Security-Policy",
    "Referrer-Policy",
    "X-Content-Type-Options",
];
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Host<'a> {
    Own,
    Named(&'a str),
    Absent,
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub body_bytes: Vec<u8>,
}

pub struct TestServer {
    port: u16,
    token: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _store: tempfile::TempDir,
}

impl TestServer {
    pub fn start() -> TestServer {
        TestServer::start_with_store(|_| {})
    }

    pub fn start_with_store(prepare: impl FnOnce(&Path)) -> TestServer {
        let store = tempfile::tempdir().unwrap();
        prepare(store.path());
        let requested = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
        let server = bind(requested, &BindPolicy::loopback()).unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let config = ApiConfig {
            store: PathBuf::from(store.path()),
            allowed_hosts: allowed_hosts(&["127.0.0.1", "localhost"], port),
            version: VERSION.to_string(),
            tokens_css: TOKENS_CSS.to_string(),
            cookie_name: cookie_name(port),
        };
        let token = Token::generate().unwrap();
        let secret = token.as_str().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            serve(server, &config, &token, &|| flag.load(Ordering::SeqCst)).unwrap();
        });
        TestServer {
            port,
            token: secret,
            stop,
            thread: Some(thread),
            _store: store,
        }
    }

    pub fn stop_and_time(mut self) -> Duration {
        let started = Instant::now();
        self.stop_within(Duration::from_secs(10));
        started.elapsed()
    }

    pub fn stop_within(&mut self, limit: Duration) -> bool {
        self.stop.store(true, Ordering::SeqCst);
        let Some(thread) = self.thread.take() else {
            return true;
        };
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            thread.join().ok();
            done.send(()).ok();
        });
        finished.recv_timeout(limit).is_ok()
    }

    pub fn store_path(&self) -> &Path {
        self._store.path()
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn cookie_name(&self) -> String {
        cookie_name(self.port)
    }

    pub fn own_host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn exchange(
        &self,
        method: &str,
        target: &str,
        host: Host,
        headers: &[(&str, &str)],
    ) -> Reply {
        let (version, host_line) = match host {
            Host::Own => ("1.1", format!("Host: {}\r\n", self.own_host())),
            Host::Named(name) => ("1.1", format!("Host: {name}\r\n")),
            Host::Absent => ("1.0", String::new()),
        };
        let extra: String = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect();
        let text = format!(
            "{method} {target} HTTP/{version}\r\n{host_line}{extra}Connection: close\r\n\r\n"
        );
        self.send_raw(text.as_bytes())
    }

    pub fn send_raw(&self, bytes: &[u8]) -> Reply {
        let mut stream = self.connect();
        stream.write_all(bytes).unwrap();
        read_reply(&mut stream)
    }

    pub fn connect(&self) -> TcpStream {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).unwrap();
        stream.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        stream.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        stream
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop_within(Duration::from_secs(2));
    }
}

pub fn read_reply(stream: &mut TcpStream) -> Reply {
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    parse_reply(&raw)
}

fn parse_reply(raw: &[u8]) -> Reply {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    let chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("Transfer-Encoding") && value.eq_ignore_ascii_case("chunked")
    });
    let body_bytes = if chunked {
        dechunk(&raw[split + 4..])
    } else {
        raw[split + 4..].to_vec()
    };
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&body_bytes).into_owned(),
        body_bytes,
    }
}

fn dechunk(mut rest: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let line_end = rest.windows(2).position(|w| w == b"\r\n").unwrap();
        let size =
            usize::from_str_radix(std::str::from_utf8(&rest[..line_end]).unwrap().trim(), 16)
                .unwrap();
        rest = &rest[line_end + 2..];
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}

impl Reply {
    pub fn redacting(mut self, swaps: &[(&str, &str)]) -> Reply {
        for (from, to) in swaps {
            for (_, value) in &mut self.headers {
                *value = value.replace(from, to);
            }
            self.body = self.body.replace(from, to);
        }
        self
    }

    pub fn golden_text(&self) -> String {
        let mut out = format!("status: {}\n", self.status);
        for name in STABLE_HEADERS {
            for (found, value) in &self.headers {
                if found.eq_ignore_ascii_case(name) {
                    out.push_str(&format!("{name}: {value}\n"));
                }
            }
        }
        out.push_str(&format!("\n{}\n", self.body));
        out
    }
}

pub fn assert_golden(name: &str, reply: &Reply) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.txt"));
    let actual = reply.golden_text();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "golden {} is missing ({error}); run with UPDATE_GOLDEN=1 to create it. Actual:\n{actual}",
            path.display()
        )
    });
    assert!(
        expected == actual,
        "golden {} differs:\n{}",
        path.display(),
        line_diff(&expected, &actual)
    );
}

fn line_diff(expected: &str, actual: &str) -> String {
    let (left, right): (Vec<&str>, Vec<&str>) =
        (expected.lines().collect(), actual.lines().collect());
    (0..left.len().max(right.len()))
        .filter(|&i| left.get(i) != right.get(i))
        .map(|i| {
            format!(
                "line {}:\n- {}\n+ {}\n",
                i + 1,
                left.get(i).copied().unwrap_or("<none>"),
                right.get(i).copied().unwrap_or("<none>")
            )
        })
        .collect()
}
