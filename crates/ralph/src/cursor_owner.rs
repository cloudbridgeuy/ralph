use ralph_core::worker::{
    caller_owns_worker, ready_path, sweep_decision, CursorFiles, OwnerFacts, OwnerRecord, Sweep,
    CURSOR_OWNER_DIR,
};
use std::fs;
use std::io;
use std::path::Path;
use std::process::{self, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) const TERM_GRACE: Duration = Duration::from_secs(3);
pub(crate) const TERM_POLL: Duration = Duration::from_millis(20);

pub(crate) fn signal_group(group: u32, signal: &str) {
    Command::new("kill")
        .args([format!("-{signal}"), "--".to_string(), format!("-{group}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

pub(crate) fn process_start(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let start = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!start.is_empty()).then_some(start)
}

fn alive_since(pid: u32, start: &str) -> bool {
    process_start(pid).is_some_and(|current| current == start)
}

fn own_ancestry() -> Vec<u32> {
    let mut pids = vec![process::id()];
    while let Some(parent) = pids.last().and_then(|pid| parent_pid(*pid)) {
        if parent <= 1 || pids.contains(&parent) {
            break;
        }
        pids.push(parent);
    }
    pids
}

fn parent_pid(pid: u32) -> Option<u32> {
    let output = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

pub fn caller_owns_cursor(store_dir: &Path, id: &str) -> bool {
    fs::read_to_string(CursorFiles::new(store_dir, id).owner)
        .ok()
        .and_then(|text| OwnerRecord::parse(&text))
        .is_some_and(|record| caller_owns_worker(record.chat_pid, &own_ancestry()))
}

pub fn owned_cursor_ids(store_dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = request_ids(store_dir, CURSOR_OWNER_DIR)
        .into_iter()
        .filter(|id| caller_owns_cursor(store_dir, id))
        .collect();
    ids.sort();
    ids
}

pub fn sweep_owner(store_dir: &Path, id: &str) -> Option<Sweep> {
    let files = CursorFiles::new(store_dir, id);
    let text = fs::read_to_string(&files.owner).ok()?;
    let record = OwnerRecord::parse(&text);
    let chat_alive = record
        .as_ref()
        .is_some_and(|r| alive_since(r.chat_pid, &r.chat_start));
    let facts = OwnerFacts {
        chat_alive,
        group_alive: record
            .as_ref()
            .is_some_and(|r| alive_since(r.group, &r.group_start)),
        mine: chat_alive && record.as_ref().is_some_and(|r| r.chat_pid == process::id()),
    };
    let decision = sweep_decision(facts);
    if let (Sweep::Reclaim { kill_group: true }, Some(record)) = (decision, &record) {
        signal_group(record.group, "TERM");
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline && alive_since(record.group, &record.group_start) {
            thread::sleep(TERM_POLL);
        }
        signal_group(record.group, "KILL");
    }
    if matches!(decision, Sweep::Reclaim { .. }) {
        for path in files.all() {
            fs::remove_file(path).ok();
        }
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_dir_all(&files.interrupt).ok();
        fs::remove_file(ready_path(store_dir, id)).ok();
    }
    Some(decision)
}

pub fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

pub(crate) fn request_ids(store_dir: &Path, dir: &str) -> Vec<String> {
    fs::read_dir(store_dir.join(dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}
