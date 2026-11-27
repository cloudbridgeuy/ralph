use super::core::{self, Screen, Shot, Theme};
use super::lines::{LineSplitter, LINE_CAP};
use super::mask;
use super::prefs;
use super::seed;
use super::webdriver::{self, Listening};
use super::BoxError;
use serde_json::Value;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::sleep;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
const STDERR_SETTLE: Duration = Duration::from_millis(200);
const EXIT_GRACE: Duration = Duration::from_secs(1);
const READ_RETRY: Duration = Duration::from_millis(50);
const READY_POLL: Duration = Duration::from_millis(100);
const SETTLE: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const LAUNCH_ATTEMPTS: u32 = 3;
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

pub struct CaptureOptions {
    pub ralph: PathBuf,
    pub firefox: PathBuf,
    pub screen: Option<Screen>,
    pub theme: Option<Theme>,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Marionette {
    stream: TcpStream,
    next_id: u32,
}

enum ConnectError {
    Exited(String),
    Failed(BoxError),
}

impl<E: Into<BoxError>> From<E> for ConnectError {
    fn from(error: E) -> Self {
        ConnectError::Failed(error.into())
    }
}

impl Marionette {
    fn connect(port: u16, firefox: &mut ChildGuard) -> Result<Self, ConnectError> {
        let started = Instant::now();
        let stream = loop {
            let last_error = match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(error) => error,
            };
            let exit = firefox.0.try_wait()?.map(|status| status.to_string());
            match webdriver::connect_step(exit.as_deref(), started.elapsed(), CONNECT_TIMEOUT) {
                webdriver::ConnectStep::Wait => sleep(READY_POLL),
                webdriver::ConnectStep::Exited(status) => return Err(ConnectError::Exited(status)),
                webdriver::ConnectStep::TimedOut => {
                    return Err(
                        format!("firefox did not open marionette on {port}: {last_error}").into(),
                    )
                }
            }
        };
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
        let mut client = Marionette { stream, next_id: 0 };
        webdriver::validate_hello(&client.read_message()?)?;
        client.call("WebDriver:NewSession", &webdriver::new_session_body())?;
        client.navigate("about:blank")?;
        Ok(client)
    }

    fn read_message(&mut self) -> Result<Value, BoxError> {
        let mut prefix = String::new();
        loop {
            let mut byte = [0u8; 1];
            self.stream.read_exact(&mut byte)?;
            if byte[0] == b':' {
                break;
            }
            if prefix.len() >= webdriver::MAX_LENGTH_DIGITS {
                return Err(webdriver::WebdriverError::BadFrameLength(prefix).into());
            }
            prefix.push(char::from(byte[0]));
        }
        let mut body = vec![0u8; webdriver::parse_length(&prefix)?];
        self.stream.read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    }

    fn call(&mut self, name: &str, params: &Value) -> Result<Value, BoxError> {
        self.next_id += 1;
        let id = self.next_id;
        self.stream
            .write_all(webdriver::command_frame(id, name, params).as_bytes())?;
        let reply = self.read_message()?;
        Ok(webdriver::command_result(&reply, id)?)
    }

    fn navigate(&mut self, url: &str) -> Result<(), BoxError> {
        self.call("WebDriver:Navigate", &webdriver::navigate_body(url))?;
        Ok(())
    }

    fn execute(&mut self, script: &str) -> Result<Value, BoxError> {
        self.call("WebDriver:ExecuteScript", &webdriver::execute_body(script))
    }

    fn wait_ready(&mut self, screen: Screen) -> Result<bool, BoxError> {
        let script = webdriver::ready_script(screen);
        let started = Instant::now();
        while started.elapsed() < READY_TIMEOUT {
            if self.execute(script)?["value"] == Value::Bool(true) {
                sleep(SETTLE);
                return Ok(true);
            }
            sleep(READY_POLL);
        }
        sleep(SETTLE);
        Ok(false)
    }

    fn mask(&mut self, screen: Screen) -> Result<(), BoxError> {
        if let Some(script) = mask::mask_script(screen) {
            self.execute(&script)?;
        }
        Ok(())
    }

    fn screenshot(&mut self) -> Result<Vec<u8>, BoxError> {
        let result = self.call("WebDriver:TakeScreenshot", &webdriver::screenshot_body())?;
        Ok(webdriver::decode_screenshot(&result)?)
    }
}

fn free_port() -> Result<u16, BoxError> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn start_ralph(
    ralph: &Path,
    store: &Path,
    home: &Path,
    cwd: &Path,
) -> Result<(ChildGuard, Listening), BoxError> {
    if !ralph.is_file() {
        return Err(format!(
            "{} does not exist; run `cargo build -p ralph` or pass --ralph",
            ralph.display()
        )
        .into());
    }
    fs::create_dir_all(cwd)?;
    let mut child = Command::new(ralph)
        .current_dir(cwd)
        .args(["web", "--store"])
        .arg(store)
        .args(["--port", "0"])
        .env_clear()
        .envs(webdriver::child_env(
            home,
            std::env::var("PATH").ok().as_deref(),
            std::env::var("TMPDIR").ok().as_deref(),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", ralph.display()))?;
    let stdout = child.stdout.take().ok_or("ralph web has no stdout")?;
    let stderr = child.stderr.take().ok_or("ralph web has no stderr")?;
    let mut guard = ChildGuard(child);
    let stderr_head = Arc::new(Mutex::new(Vec::new()));
    spawn_stderr_reader(stderr, Arc::clone(&stderr_head));
    let stdout_tail = Arc::new(Mutex::new(Vec::new()));
    let lines = spawn_line_reader(stdout, Arc::clone(&stdout_tail));
    let started = Instant::now();
    let mut read = Vec::new();
    let reason = loop {
        let left = LISTEN_TIMEOUT.saturating_sub(started.elapsed());
        match lines.recv_timeout(left) {
            Ok(line) => {
                read.push(line);
                if let Some(listening) = webdriver::find_listening(&read) {
                    return Ok((guard, listening));
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                break format!("no listen line within {}s", LISTEN_TIMEOUT.as_secs())
            }
            Err(RecvTimeoutError::Disconnected) => break "stdout closed".to_string(),
        }
    };
    Err(start_failure(&mut guard, &reason, &stdout_tail, &stderr_head).into())
}

fn spawn_line_reader(mut stdout: ChildStdout, tail: Arc<Mutex<Vec<u8>>>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut splitter = LineSplitter::new(LINE_CAP);
        let mut chunk = [0u8; 4096];
        loop {
            let count = match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    sleep(READ_RETRY);
                    continue;
                }
            };
            for line in splitter.push_bytes(&chunk[..count]) {
                record_line(&tail, &sender, line);
            }
        }
        if let Some(line) = splitter.finish() {
            record_line(&tail, &sender, line);
        }
    });
    receiver
}

fn record_line(tail: &Mutex<Vec<u8>>, sender: &mpsc::Sender<String>, line: String) {
    if let Ok(mut buf) = tail.lock() {
        webdriver::push_tail(&mut buf, line.as_bytes(), webdriver::FAILURE_STREAM_CAP);
        webdriver::push_tail(&mut buf, b"\n", webdriver::FAILURE_STREAM_CAP);
    }
    let _ = sender.send(line);
}

fn spawn_stderr_reader(mut stderr: ChildStderr, head: Arc<Mutex<Vec<u8>>>) {
    thread::spawn(move || {
        let mut chunk = [0u8; 1024];
        while let Ok(count) = stderr.read(&mut chunk) {
            if count == 0 {
                break;
            }
            if let Ok(mut buf) = head.lock() {
                webdriver::push_head(&mut buf, &chunk[..count], webdriver::FAILURE_STREAM_CAP);
            }
        }
    });
}

fn exit_within(child: &mut Child, grace: Duration) -> Option<ExitStatus> {
    let started = Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if started.elapsed() >= grace {
            return None;
        }
        sleep(Duration::from_millis(20));
    }
}

fn start_failure(
    guard: &mut ChildGuard,
    reason: &str,
    stdout_tail: &Mutex<Vec<u8>>,
    stderr_head: &Mutex<Vec<u8>>,
) -> String {
    let status = match exit_within(&mut guard.0, EXIT_GRACE) {
        Some(status) => format!("{reason}; exited with {status}"),
        None => {
            let _ = guard.0.kill();
            match guard.0.wait() {
                Ok(status) => format!("{reason}; killed, {status}"),
                Err(error) => format!("{reason}; wait failed: {error}"),
            }
        }
    };
    sleep(STDERR_SETTLE);
    let text = |shared: &Mutex<Vec<u8>>| {
        shared
            .lock()
            .map(|buf| String::from_utf8_lossy(&buf).into_owned())
            .unwrap_or_default()
    };
    webdriver::format_start_failure(&status, &text(stdout_tail), &text(stderr_head))
}

fn start_firefox(firefox: &Path, profile: &Path) -> Result<ChildGuard, BoxError> {
    let child = Command::new(firefox)
        .args(["--headless", "--no-remote", "--marionette", "--profile"])
        .arg(profile)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", firefox.display()))?;
    Ok(ChildGuard(child))
}

fn clear_stale(dir: &Path, shots: &[Shot]) -> Result<(), BoxError> {
    fs::create_dir_all(dir)?;
    for name in core::stale_names(shots, &super::png_names(dir)?) {
        fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

fn launch_firefox(
    firefox: &Path,
    theme: Theme,
    profile: &Path,
) -> Result<(ChildGuard, Marionette), BoxError> {
    for attempt in 1..=LAUNCH_ATTEMPTS {
        let port = free_port()?;
        fs::write(profile.join("user.js"), prefs::user_js(theme, port))?;
        let mut child = start_firefox(firefox, profile)?;
        match Marionette::connect(port, &mut child) {
            Ok(browser) => return Ok((child, browser)),
            Err(ConnectError::Exited(status)) if attempt < LAUNCH_ATTEMPTS => {
                println!("firefox exited early ({status}); launching again");
            }
            Err(ConnectError::Exited(status)) => {
                return Err(format!(
                "firefox exited before opening marionette ({status}) on {LAUNCH_ATTEMPTS} launches"
            )
                .into())
            }
            Err(ConnectError::Failed(error)) => return Err(error),
        }
    }
    Err("firefox was not launched".into())
}

fn capture_theme(
    options: &CaptureOptions,
    listening: &Listening,
    group: &[Shot],
    scratch: &Path,
    out: &Path,
) -> Result<Vec<String>, BoxError> {
    let theme = group[0].theme;
    let profile = scratch.join(format!("profile-{}", theme.name()));
    fs::create_dir_all(&profile)?;
    let (_firefox, mut browser) = launch_firefox(&options.firefox, theme, &profile)?;
    if !webdriver::scheme_matches(theme, &browser.execute(webdriver::SCHEME_SCRIPT)?) {
        return Err(format!("the browser did not switch to the {} scheme", theme.name()).into());
    }
    let mut timed_out = Vec::new();
    browser.navigate(&webdriver::token_url(listening))?;
    for shot in group {
        browser.call("WebDriver:SetWindowRect", &webdriver::rect_body(shot.width))?;
        browser.navigate("about:blank")?;
        browser.navigate(&webdriver::shot_url(&listening.base, shot.screen))?;
        if !browser.wait_ready(shot.screen)? {
            timed_out.push(shot.name());
        }
        browser.mask(shot.screen)?;
        sleep(SETTLE);
        browser.mask(shot.screen)?;
        fs::write(out.join(shot.name()), browser.screenshot()?)?;
        println!("captured {}", shot.name());
    }
    Ok(timed_out)
}

pub fn capture(options: &CaptureOptions, out: &Path) -> Result<Vec<String>, BoxError> {
    let shots = core::filter(core::matrix(), options.screen, options.theme);
    let scratch = tempfile::tempdir()?;
    let store = scratch.path().join("store");
    seed::seed(&store)?;
    let home = scratch.path().join("home");
    fs::create_dir_all(&home)?;
    let (_ralph, listening) =
        start_ralph(&options.ralph, &store, &home, &scratch.path().join("cwd"))?;
    clear_stale(out, &shots)?;
    let mut timed_out = Vec::new();
    for theme in Theme::ALL {
        let group: Vec<Shot> = shots
            .iter()
            .copied()
            .filter(|shot| shot.theme == theme)
            .collect();
        if !group.is_empty() {
            timed_out.extend(capture_theme(
                options,
                &listening,
                &group,
                scratch.path(),
                out,
            )?);
        }
    }
    Ok(timed_out)
}
