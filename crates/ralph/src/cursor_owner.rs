use crate::process_tree::group_table;
use crate::transport::stdio::stop_group;
use crate::warn::warn;
use ralph_core::worker::{
    agent_held, caller_owns_worker, cursor_group_reclaimable, group_live, interrupt_result_text,
    sweep_decision, HandoffFiles, OwnerFacts, OwnerRecord, Sweep, CODEX_OWNER_DIR,
    CODEX_SESSION_DIR, CURSOR_OWNER_DIR, CURSOR_SESSION_DIR,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

pub fn report_start_warning(warning: String, spawn_file: Option<&Path>) {
    match spawn_file {
        Some(path) if write_file(path, &warning).is_ok() => {}
        _ => warn(warning),
    }
}

pub fn caller_owns(files: &HandoffFiles) -> bool {
    fs::read_to_string(&files.owner)
        .ok()
        .and_then(|text| OwnerRecord::parse(&text))
        .is_some_and(|record| caller_owns_worker(record.chat_pid, &own_ancestry()))
}

fn owned_ids(store_dir: &Path, dir: &str, files: fn(&Path, &str) -> HandoffFiles) -> Vec<String> {
    let mut ids: Vec<String> = request_ids(store_dir, dir)
        .into_iter()
        .filter(|id| caller_owns(&files(store_dir, id)))
        .collect();
    ids.sort();
    ids
}

pub fn owned_cursor_ids(store_dir: &Path) -> Vec<String> {
    owned_ids(store_dir, CURSOR_OWNER_DIR, HandoffFiles::cursor)
}

pub fn owned_codex_ids(store_dir: &Path) -> Vec<String> {
    owned_ids(store_dir, CODEX_OWNER_DIR, HandoffFiles::codex)
}

pub fn acp_alive_ids(store_dir: &Path) -> Vec<String> {
    let mut ids: Vec<String> = request_ids(store_dir, CURSOR_OWNER_DIR)
        .into_iter()
        .filter(|id| {
            owner_facts(&HandoffFiles::cursor(store_dir, id).owner)
                .is_some_and(|(_, facts)| agent_held(facts))
        })
        .collect();
    ids.sort();
    ids
}

pub fn resumable_ids(store_dir: &Path) -> Vec<String> {
    let mut ids = request_ids(store_dir, CURSOR_SESSION_DIR);
    ids.extend(request_ids(store_dir, CODEX_SESSION_DIR));
    ids.sort();
    ids
}

fn owner_facts(owner: &Path) -> Option<(Option<OwnerRecord>, OwnerFacts)> {
    let text = fs::read_to_string(owner).ok()?;
    let record = OwnerRecord::parse(&text);
    let chat_alive = record
        .as_ref()
        .is_some_and(|r| alive_since(r.chat_pid, &r.chat_start));
    let facts = OwnerFacts {
        chat_alive,
        group_alive: record.as_ref().is_some_and(group_alive),
        mine: chat_alive && record.as_ref().is_some_and(|r| r.chat_pid == process::id()),
    };
    Some((record, facts))
}

fn group_alive(record: &OwnerRecord) -> bool {
    let leader = process_start(record.group);
    let reclaimable = leader.is_none() && cursor_group_reclaimable(&group_table(), record.group);
    group_live(leader.as_deref(), &record.group_start, reclaimable)
}

pub fn sweep_owner(files: &HandoffFiles) -> Option<Sweep> {
    let (record, facts) = owner_facts(&files.owner)?;
    let decision = sweep_decision(facts);
    if let (Sweep::Reclaim { kill_group: true }, Some(record)) = (decision, &record) {
        stop_group(record.group, || {
            alive_since(record.group, &record.group_start)
        });
    }
    if matches!(decision, Sweep::Reclaim { .. }) {
        for path in files.all() {
            fs::remove_file(path).ok();
        }
        fs::remove_dir_all(&files.prompt).ok();
        fs::remove_dir_all(&files.interrupt).ok();
        fs::remove_dir_all(&files.interrupt_result).ok();
    }
    Some(decision)
}

pub fn write_owner_record(owner: &Path, group: u32, agent: &str) -> Result<(), String> {
    let chat = process::id();
    let record = OwnerRecord {
        chat_pid: chat,
        chat_start: process_start(chat).ok_or("could not read the chat start time")?,
        group,
        group_start: process_start(group)
            .ok_or_else(|| format!("could not read the {agent} agent start time"))?,
    };
    write_file(owner, &record.render())
        .map_err(|e| format!("could not write the {agent} owner file: {e}"))
}

pub fn claim_owner(owner: &Path) -> io::Result<()> {
    let chat = process::id();
    let start = process_start(chat)
        .ok_or_else(|| io::Error::other("could not read the chat start time"))?;
    let record = OwnerRecord {
        chat_pid: chat,
        chat_start: start.clone(),
        group: chat,
        group_start: start,
    };
    let name = owner.file_name().unwrap_or_default().to_string_lossy();
    let pending = owner
        .parent()
        .and_then(Path::parent)
        .unwrap_or(owner)
        .join(format!(".claim-{name}-{chat}"));
    write_file(&pending, &record.render())?;
    if let Some(dir) = owner.parent() {
        fs::create_dir_all(dir)?;
    }
    let linked = fs::hard_link(&pending, owner);
    fs::remove_file(&pending).ok();
    linked
}

pub fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

pub fn write_interrupt_result(results: &Path, entry: &str, cancelled: bool) {
    let pending = results.join(format!(".{entry}"));
    if write_file(&pending, interrupt_result_text(cancelled)).is_ok() {
        fs::rename(&pending, results.join(entry)).ok();
    }
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

pub(crate) fn queued_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_owner_succeeds_once_and_leaves_a_parseable_record() {
        let store = tempfile::tempdir().unwrap();
        let owner = store.path().join("codex").join("A");
        claim_owner(&owner).unwrap();
        let second = claim_owner(&owner).unwrap_err();
        assert_eq!(second.kind(), io::ErrorKind::AlreadyExists);
        let record = OwnerRecord::parse(&fs::read_to_string(&owner).unwrap()).unwrap();
        assert_eq!(record.chat_pid, process::id());
        assert_eq!(fs::read_dir(store.path()).unwrap().count(), 1);
    }
}
