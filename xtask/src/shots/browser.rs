use super::core::{self, Screen, Shot, Theme};
use super::mask;
use super::seed;
use super::webdriver::{self, Listening};
use super::BoxError;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const READY_TIMEOUT: Duration = Duration::from_secs(10);
const READY_POLL: Duration = Duration::from_millis(100);
const SETTLE: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
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

impl Marionette {
    fn connect(port: u16) -> Result<Self, BoxError> {
        let started = Instant::now();
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(error) if started.elapsed() >= CONNECT_TIMEOUT => {
                    return Err(
                        format!("firefox did not open marionette on {port}: {error}").into(),
                    )
                }
                Err(_) => sleep(READY_POLL),
            }
        };
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        let mut client = Marionette { stream, next_id: 0 };
        client.read_message()?;
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
) -> Result<(ChildGuard, Listening), BoxError> {
    if !ralph.is_file() {
        return Err(format!(
            "{} does not exist; run `cargo build -p ralph` or pass --ralph",
            ralph.display()
        )
        .into());
    }
    let mut child = Command::new(ralph)
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
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start {}: {error}", ralph.display()))?;
    let stdout = child.stdout.take().ok_or("ralph web has no stdout")?;
    let guard = ChildGuard(child);
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line)?;
    Ok((guard, webdriver::parse_listening(&line)?))
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

fn clear_pngs(dir: &Path) -> Result<(), BoxError> {
    fs::create_dir_all(dir)?;
    for entry in fs::read_dir(dir)?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "png") {
            fs::remove_file(path)?;
        }
    }
    Ok(())
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
    let port = free_port()?;
    fs::write(profile.join("user.js"), webdriver::user_js(theme, port))?;
    let _firefox = start_firefox(&options.firefox, &profile)?;
    let mut browser = Marionette::connect(port)?;
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
    let (_ralph, listening) = start_ralph(&options.ralph, &store, &home)?;
    clear_pngs(out)?;
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
