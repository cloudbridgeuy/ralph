use crate::config::{AppConfig, MermaidSection};
use crate::render::mermaid::{fits_width, mermaid_precheck};
use crate::warn::warn;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(target_os = "macos")]
fn rss_bytes(pid: u32) -> Option<u64> {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            std::ptr::addr_of_mut!(info).cast(),
            size,
        )
    };
    (written == size).then_some(info.pti_resident_size)
}

#[cfg(target_os = "linux")]
fn rss_bytes(pid: u32) -> Option<u64> {
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    Some(pages * page_size)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn rss_bytes(_pid: u32) -> Option<u64> {
    None
}

fn over_rss_limit(child: &Child, max_rss_mb: u64) -> bool {
    rss_bytes(child.id()).is_some_and(|b| b > max_rss_mb.saturating_mul(1024 * 1024))
}

fn spawn_child(width: usize) -> Option<Child> {
    Command::new(std::env::current_exe().ok()?)
        .arg("__mermaid")
        .arg("--width")
        .arg(width.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn run_guarded(source: &str, width: usize, limits: &MermaidSection) -> Option<String> {
    let mut child = spawn_child(width)?;
    let mut stdin = child.stdin.take()?;
    let mut stdout = child.stdout.take()?;
    let input = source.to_string();
    thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).ok().map(|_| buf)
    });
    let deadline = Instant::now() + Duration::from_millis(limits.timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline && !over_rss_limit(&child, limits.max_rss_mb) => {
                thread::sleep(POLL_INTERVAL)
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = reader.join().ok()??;
    if !status?.success() {
        return None;
    }
    String::from_utf8(output).ok()
}

fn limits() -> &'static MermaidSection {
    static LIMITS: OnceLock<MermaidSection> = OnceLock::new();
    LIMITS.get_or_init(|| match AppConfig::load() {
        Ok(config) => config.mermaid,
        Err(e) => {
            warn(format!("{e}; using default [mermaid] limits"));
            MermaidSection::default()
        }
    })
}

pub fn render(source: &str, width: usize) -> Option<String> {
    let limits = limits();
    mermaid_precheck(source, limits).ok()?;
    let output = run_guarded(source, width, limits)?;
    fits_width(&output, width).then_some(output.trim_end().to_string())
}
