use crate::cursor_owner::{process_start, TERM_GRACE, TERM_POLL};
use ralph_core::worker::{
    descendants, parse_group_rows, parse_ps, still_running, GroupRow, Tracked,
};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

pub(crate) fn process_table() -> Vec<(u32, u32)> {
    Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| parse_ps(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

pub(crate) fn group_table() -> Vec<GroupRow> {
    Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid=,command="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| parse_group_rows(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

pub(crate) fn snapshot_tree(root: u32) -> Vec<Tracked> {
    descendants(&process_table(), root)
        .into_iter()
        .filter_map(|pid| process_start(pid).map(|start| Tracked { pid, start }))
        .collect()
}

fn signal_pid(pid: u32, signal: &str) {
    Command::new("kill")
        .args([format!("-{signal}"), "--".to_string(), pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

pub(crate) fn sweep_tree(snapshot: &[Tracked]) -> Vec<u32> {
    let found = still_running(snapshot, process_start);
    for pid in &found {
        signal_pid(*pid, "TERM");
    }
    let deadline = Instant::now() + TERM_GRACE;
    while Instant::now() < deadline && !still_running(snapshot, process_start).is_empty() {
        thread::sleep(TERM_POLL);
    }
    for pid in still_running(snapshot, process_start) {
        signal_pid(pid, "KILL");
    }
    let deadline = Instant::now() + TERM_GRACE;
    let mut survivors = still_running(snapshot, process_start);
    while Instant::now() < deadline && !survivors.is_empty() {
        thread::sleep(TERM_POLL);
        survivors = still_running(snapshot, process_start);
    }
    survivors
}
