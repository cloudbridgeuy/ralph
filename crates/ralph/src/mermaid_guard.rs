use crate::config::MermaidStyleExt;
use crate::render::mermaid::{mermaid_precheck, picture_precheck, widest};
use crate::render::mermaid_picture::{PictureRequest, Target};
use crate::ui_theme::UiColors;
use ralph_core::mermaid_file::{diagram_name, Miss};
use ralph_core::mermaid_fit::Room;
use ralph_server::config::{AppConfig, MermaidSection};
use ralph_server::warn::warn;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};
use syntect::highlighting::Theme;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const FILE_SCALE: f64 = 2.0;

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

#[derive(Clone, Copy)]
struct StdoutCap(u64);

const TEXT_STDOUT_CAP: StdoutCap = StdoutCap(4 * 1024 * 1024);
const PICTURE_STDOUT_CAP: StdoutCap = StdoutCap(16 * 1024 * 1024);

fn read_capped(stdout: impl Read, cap: StdoutCap) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    stdout
        .take(cap.0.saturating_add(1))
        .read_to_end(&mut buf)
        .ok()?;
    (buf.len() as u64 <= cap.0).then_some(buf)
}

struct Guarded<'a> {
    args: &'a [&'a str],
    input: Vec<u8>,
    timeout_ms: u64,
    max_rss_mb: u64,
    stdout_cap: StdoutCap,
}

fn spawn_child(args: &[&str]) -> Option<Child> {
    Command::new(std::env::current_exe().ok()?)
        .arg("__mermaid")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

fn run_guarded(job: Guarded<'_>) -> Option<Vec<u8>> {
    let mut child = spawn_child(job.args)?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let cap = job.stdout_cap;
    let input = job.input;
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let reader = thread::spawn(move || read_capped(stdout, cap));
    let deadline = Instant::now() + Duration::from_millis(job.timeout_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline && !over_rss_limit(&child, job.max_rss_mb) => {
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
    status?.success().then_some(output)
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

pub fn render(source: &str, width: usize) -> Result<String, Miss> {
    let limits = limits();
    mermaid_precheck(source, limits).map_err(|_| Miss::NoText)?;
    let width_arg = width.to_string();
    let output = run_guarded(Guarded {
        args: &["--width", &width_arg],
        input: source.as_bytes().to_vec(),
        timeout_ms: limits.timeout_ms,
        max_rss_mb: limits.max_rss_mb,
        stdout_cap: TEXT_STDOUT_CAP,
    })
    .ok_or(Miss::NoText)?;
    let output = String::from_utf8(output).map_err(|_| Miss::NoText)?;
    match widest(&output) {
        needs if needs > width => Err(Miss::TooWide { needs, has: width }),
        _ => Ok(output.trim_end().to_string()),
    }
}

pub fn picture(source: &str, theme: &Theme, ui: &UiColors, room: Room) -> Option<Vec<u8>> {
    let limits = limits();
    picture_precheck(source, limits).ok()?;
    let request = PictureRequest {
        source: source.to_string(),
        style: limits.style.resolve(theme, ui).0,
        target: Target::Pane(room),
    };
    let png = run_guarded(Guarded {
        args: &["--picture"],
        input: serde_json::to_vec(&request).ok()?,
        timeout_ms: limits.picture_timeout_ms,
        max_rss_mb: limits.max_rss_mb,
        stdout_cap: PICTURE_STDOUT_CAP,
    })?;
    (!png.is_empty()).then_some(png)
}

pub fn full_size(source: &str, theme: &Theme, ui: &UiColors, dir: &Path) -> Option<PathBuf> {
    let limits = limits();
    picture_precheck(source, limits).ok()?;
    let target = Target::File { scale: FILE_SCALE };
    let style = limits.style.resolve(theme, ui).0;
    let name = diagram_name(source, &serde_json::to_vec(&(&style, &target)).ok()?);
    let path = dir.join(&name);
    if path.is_file() {
        return Some(path);
    }
    let request = PictureRequest {
        source: source.to_string(),
        style,
        target,
    };
    let png = run_guarded(Guarded {
        args: &["--picture"],
        input: serde_json::to_vec(&request).ok()?,
        timeout_ms: limits.picture_timeout_ms,
        max_rss_mb: limits.max_rss_mb,
        stdout_cap: PICTURE_STDOUT_CAP,
    })
    .filter(|png| !png.is_empty())?;
    std::fs::create_dir_all(dir).ok()?;
    let partial = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let saved = std::fs::write(&partial, &png).and_then(|()| std::fs::rename(&partial, &path));
    if saved.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    saved.ok().map(|()| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_up_to_the_cap_is_kept() {
        let kept = read_capped(&b"abcd"[..], StdoutCap(4));
        assert_eq!(kept, Some(b"abcd".to_vec()));
    }

    #[test]
    fn output_over_the_cap_is_refused() {
        assert_eq!(read_capped(&b"abcde"[..], StdoutCap(4)), None);
        let flood = std::io::repeat(b'x').take(1 << 20);
        assert_eq!(read_capped(flood, StdoutCap(4)), None);
    }
}
