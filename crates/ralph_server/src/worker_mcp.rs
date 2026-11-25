use ralph_core::inbox::{
    decode_message, drop_log_line, DecodeError, Entry, Ledger, Position, Selection,
};
use ralph_core::mcp::tool_result;
use ralph_core::worker::{
    accept_report, append_status, decode_statuses, encode_statuses, handle_request,
    inbox_file_name, refuses_repeat_report, report_event, reported_path, sent_path, status_path,
    stop_activity_state, stop_block_output, stop_decision, unreported_path, ActivityMark,
    ActivityState, Outcome, Report, ReportFacts, ReportStatus, StatusLine, StopDecision,
    REPEAT_REPORT_REFUSAL,
};
use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use tempfile::NamedTempFile;

use crate::cursor_owner::{now_secs, write_file};
use crate::outcome_log;
use crate::worker_changes::WorkerChanges;
use crate::worker_usage::{record_or_warn, record_verify_or_warn, UsageTarget};

const DROP_LOG: &str = "inbox-drops.jsonl";
const REJECTED_DIR: &str = "rejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadMode {
    ReadOnly,
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedgerState {
    Unchanged,
    Repaired,
}

struct LoadedLedger {
    ledger: Ledger,
    state: LedgerState,
}

pub struct Inbox {
    dir: PathBuf,
    owner: Option<PathBuf>,
    once_per_turn: bool,
    lock: Mutex<()>,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            owner: None,
            once_per_turn: false,
            lock: Mutex::new(()),
        }
    }

    pub fn requiring_owner(mut self, owner: Option<PathBuf>) -> Self {
        self.owner = owner;
        self
    }

    pub fn once_per_turn(mut self, once: bool) -> Self {
        self.once_per_turn = once;
        self
    }

    fn accepts_report(&self) -> bool {
        accept_report(
            self.owner.is_some(),
            self.owner.as_ref().is_some_and(|owner| owner.exists()),
        )
    }

    pub fn post(&self, report: &Report, seq: u64) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut file = NamedTempFile::new_in(&self.dir)?;
        file.write_all(&serde_json::to_vec(report)?)?;
        let mut next = seq;
        loop {
            match file.persist_noclobber(self.dir.join(inbox_file_name(report, next))) {
                Ok(_) => return Ok(()),
                Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => file = e.file,
                Err(e) => return Err(e.error),
            }
            next += 1;
        }
    }

    fn reported_this_turn(&self, worker: &str) -> bool {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        refuses_repeat_report(self.once_per_turn, reported_path(store, worker).exists())
    }

    fn log_report(&self, report: &Report, status: ReportStatus, seq: u64) {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        outcome_log::record(store, &report.worker, |key| {
            report_event(&ReportFacts {
                key,
                status,
                text_bytes: report.text.len(),
                seq,
            })
        });
    }

    fn set_status(&self, worker: &str, text: &str) -> io::Result<()> {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        let path = status_path(store, worker);
        let dir = path.parent().unwrap_or(store);
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{worker}.{}.tmp", std::process::id()));
        let history = fs::read_to_string(&path)
            .map(|raw| decode_statuses(&raw))
            .unwrap_or_default();
        let status = StatusLine {
            at: chrono::Utc::now().to_rfc3339(),
            text: text.to_string(),
        };
        fs::write(&tmp, encode_statuses(&append_status(history, status)))?;
        fs::rename(tmp, &path)?;
        if !self.accepts_report() {
            remove_if_present(&path)?;
        }
        Ok(())
    }

    fn mark_reported(&self, worker: &str) -> io::Result<()> {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        let marker = reported_path(store, worker);
        if let Some(dir) = marker.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(marker, "")?;
        remove_if_present(&sent_path(store, worker))?;
        remove_if_present(&unreported_path(store, worker))
    }

    fn pending(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = match fs::read_dir(&self.dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok())
                .filter(|e| !e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .filter(|p| is_message(p))
                .collect::<Vec<_>>(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        paths.sort();
        Ok(paths)
    }

    fn ledger_path(&self) -> PathBuf {
        self.dir.with_extension("json")
    }

    fn copy_rejected_ledger(&self, bytes: &[u8]) -> io::Result<PathBuf> {
        let rejected_dir = self.dir.join(REJECTED_DIR);
        fs::create_dir_all(&rejected_dir)?;
        let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S%9f");
        let dest = rejected_dir.join(format!("inbox-{stamp}.json"));
        fs::write(&dest, bytes)?;
        Ok(dest)
    }

    fn load_ledger_with(
        &self,
        mode: LoadMode,
        warn: &mut impl FnMut(String),
    ) -> io::Result<LoadedLedger> {
        let path = self.ledger_path();
        match fs::read(&path) {
            Ok(bytes) => {
                let (ledger, skipped) = Ledger::decode(&bytes);
                if skipped.is_empty() {
                    return Ok(LoadedLedger {
                        ledger,
                        state: LedgerState::Unchanged,
                    });
                }
                let (note, state) = match mode {
                    LoadMode::Repair => match self.copy_rejected_ledger(&bytes) {
                        Ok(dest) => (
                            format!(" (original copied to {})", dest.display()),
                            LedgerState::Repaired,
                        ),
                        Err(err) => (
                            format!(" (could not copy original to rejected: {err})"),
                            LedgerState::Unchanged,
                        ),
                    },
                    LoadMode::ReadOnly => (String::new(), LedgerState::Unchanged),
                };
                for entry in skipped {
                    warn(format!(
                        "skipping malformed inbox ledger {}{note}: {entry}",
                        path.display()
                    ));
                }
                Ok(LoadedLedger { ledger, state })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(LoadedLedger {
                ledger: Ledger::default(),
                state: LedgerState::Unchanged,
            }),
            Err(e) => Err(e),
        }
    }

    fn load_ledger(&self) -> io::Result<Ledger> {
        Ok(self
            .load_ledger_with(LoadMode::ReadOnly, &mut |message| {
                crate::warn::warn(message)
            })?
            .ledger)
    }

    fn save_ledger(&self, ledger: &Ledger) -> io::Result<()> {
        let path = self.ledger_path();
        let tmp = path.with_file_name(".inbox.json.tmp");
        fs::write(&tmp, serde_json::to_vec(ledger)?)?;
        fs::rename(tmp, path)
    }

    fn reject_message(&self, path: &Path) -> io::Result<PathBuf> {
        let rejected_dir = self.dir.join(REJECTED_DIR);
        fs::create_dir_all(&rejected_dir)?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
        let dest = rejected_dir.join(name);
        fs::rename(path, &dest)?;
        Ok(dest)
    }

    fn reject_and_warn(&self, path: &Path, reason: &DecodeError, warn: &mut impl FnMut(String)) {
        match self.reject_message(path) {
            Ok(dest) => warn(format!(
                "moved malformed inbox file {} to {}: {reason}",
                path.display(),
                dest.display()
            )),
            Err(move_err) => warn(format!(
                "skipping malformed inbox file {} (could not move to rejected: {move_err}): {reason}",
                path.display()
            )),
        }
    }

    fn sync_with<T>(
        &self,
        warn: &mut impl FnMut(String),
        act: impl FnOnce(&mut Ledger) -> (T, bool),
    ) -> io::Result<T> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let LoadedLedger { mut ledger, state } = self.load_ledger_with(LoadMode::Repair, warn)?;
        let pending = self.pending()?;
        let mut arrivals = Vec::new();
        let mut accepted = Vec::new();
        for path in pending {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            match decode_message(&bytes) {
                Ok(report) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    arrivals.push((name, report));
                    accepted.push(path);
                }
                Err(reason) => self.reject_and_warn(&path, &reason, warn),
            }
        }
        let ingested = !accepted.is_empty();
        ledger.ingest(arrivals);
        let (value, changed) = act(&mut ledger);
        let should_save = match state {
            LedgerState::Repaired => true,
            LedgerState::Unchanged => ingested || changed,
        };
        if should_save {
            self.save_ledger(&ledger)?;
        }
        for path in accepted {
            remove_if_present(&path)?;
        }
        Ok(value)
    }

    fn sync<T>(&self, act: impl FnOnce(&mut Ledger) -> (T, bool)) -> io::Result<T> {
        self.sync_with(&mut |message| crate::warn::warn(message), act)
    }

    pub fn peek(&self) -> io::Result<Vec<Report>> {
        self.sync(|ledger| (ledger.reports(), false))
    }

    pub fn unread_count(&self) -> io::Result<usize> {
        Ok(self.load_ledger()?.reports().len() + self.pending()?.len())
    }

    pub fn drain(&self, selection: Selection) -> io::Result<Vec<Report>> {
        self.sync(|ledger| {
            let reports = ledger.take(selection);
            let changed = !reports.is_empty();
            (reports, changed)
        })
    }

    pub fn drain_closes(&self) -> io::Result<Vec<Report>> {
        self.sync(|ledger| {
            let reports = ledger.take_closes();
            let changed = !reports.is_empty();
            (reports, changed)
        })
    }

    pub fn drop_report(&self, position: Position) -> io::Result<Option<Entry>> {
        let log = self.dir.with_file_name(DROP_LOG);
        self.sync(|ledger| match ledger.get(position).cloned() {
            None => (Ok(None), false),
            Some(entry) => match append_line(&log, &drop_log_line(&entry, chrono::Utc::now())) {
                Err(e) => (Err(e), false),
                Ok(()) => {
                    ledger.remove(position);
                    (Ok(Some(entry)), true)
                }
            },
        })?
    }

    pub fn drop_worker(&self, worker: &str) -> io::Result<usize> {
        let log = self.dir.with_file_name(DROP_LOG);
        self.sync(|ledger| {
            let dropped = ledger.drop_worker(worker);
            let now = chrono::Utc::now();
            let logged = dropped
                .iter()
                .try_for_each(|entry| append_line(&log, &drop_log_line(entry, now)));
            (logged.map(|()| dropped.len()), !dropped.is_empty())
        })?
    }

    pub fn ledger(&self) -> io::Result<Ledger> {
        self.sync(|ledger| (ledger.clone(), false))
    }
}

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")?;
    file.sync_all()
}

fn is_message(path: &Path) -> bool {
    path.parent()
        .and_then(|parent| parent.file_name())
        .is_none_or(|name| name != REJECTED_DIR)
        && path.extension().is_some_and(|ext| ext == "json")
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| !n.starts_with('.'))
}

pub fn run_stdio_server(
    worker: &str,
    inbox: &Inbox,
    changes: Option<&WorkerChanges>,
    input: impl BufRead,
    mut output: impl Write,
) -> io::Result<()> {
    let mut seq = 0;
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                crate::warn::warn(format!("ignoring malformed JSON-RPC line: {e}"));
                continue;
            }
        };
        let response = match handle_request(&request) {
            Outcome::Silent => continue,
            Outcome::Respond(v) => v,
            Outcome::Status { id, text } => {
                if !inbox.accepts_report() {
                    tool_result(id, "Could not set status: worker closed.", true)
                } else {
                    match inbox.set_status(worker, &text) {
                        Ok(()) => tool_result(id, "Status set.", false),
                        Err(e) => tool_result(id, &format!("Could not set status: {e}"), true),
                    }
                }
            }
            Outcome::Report { id, status, text } => {
                if !inbox.accepts_report() {
                    let closed = tool_result(id, "Could not deliver: worker closed.", true);
                    writeln!(output, "{closed}")?;
                    output.flush()?;
                    continue;
                }
                if inbox.reported_this_turn(worker) {
                    let refused = tool_result(id, REPEAT_REPORT_REFUSAL, true);
                    writeln!(output, "{refused}")?;
                    output.flush()?;
                    continue;
                }
                seq += 1;
                let report = Report {
                    worker: worker.to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                    status: status.as_str().to_string(),
                    text: changes.map_or(text.clone(), |changes| changes.attach_to(&text)),
                };
                match inbox.post(&report, seq) {
                    Ok(()) => {
                        inbox.log_report(&report, status, seq);
                        if let Err(e) = inbox.mark_reported(worker) {
                            crate::warn::warn(format!("could not write the reported marker: {e}"));
                        }
                        tool_result(
                            id,
                            "Delivered. Stop and wait for the person's answer.",
                            false,
                        )
                    }
                    Err(e) => tool_result(id, &format!("Could not deliver: {e}"), true),
                }
            }
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

fn write_activity(path: Option<&Path>, state: ActivityState) {
    let Some(path) = path else { return };
    let mark = ActivityMark {
        at: now_secs(),
        state,
    };
    if let Err(e) = write_file(path, &mark.render()) {
        crate::warn::warn(format!("could not write the activity marker: {e}"));
    }
}

pub(crate) fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

pub struct StopHook<'a> {
    pub reported: &'a Path,
    pub unreported: &'a Path,
    pub activity: Option<&'a Path>,
    pub changes: Option<&'a WorkerChanges>,
    pub usage: Option<&'a UsageTarget>,
    pub verify: bool,
}

pub fn run_stop_hook(
    hook: &StopHook,
    mut input: impl Read,
    mut output: impl Write,
) -> io::Result<()> {
    let mut payload = String::new();
    if let Err(e) = input.read_to_string(&mut payload) {
        crate::warn::warn(format!(
            "letting the worker stop: unreadable Stop hook input: {e}"
        ));
        write_activity(hook.activity, ActivityState::Idle);
        return Ok(());
    }
    let turn_id = hook.changes.and_then(WorkerChanges::snapshot_turn_or_warn);
    if let Some(usage) = hook.usage {
        if hook.verify {
            record_verify_or_warn(usage, &payload);
        } else {
            record_or_warn(usage, &payload, turn_id);
        }
    }
    let StopHook {
        reported,
        unreported,
        activity,
        ..
    } = *hook;
    let decision = stop_decision(&payload, reported.exists());
    write_activity(activity, stop_activity_state(&decision));
    match decision {
        StopDecision::AllowBackground => {}
        StopDecision::GiveUp => {
            if let Some(dir) = unreported.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(unreported, "")?;
        }
        StopDecision::ConsumeMarker => {
            remove_if_present(reported)?;
            remove_if_present(unreported)?;
        }
        StopDecision::Block => writeln!(output, "{}", stop_block_output())?,
        StopDecision::FailOpen(reason) => {
            crate::warn::warn(format!("letting the worker stop: {reason}"))
        }
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop_hook(reported: &Path, payload: &str) -> String {
        let unreported = reported.with_file_name("unreported");
        let mut out = Vec::new();
        let hook = StopHook {
            reported,
            unreported: &unreported,
            activity: None,
            changes: None,
            usage: None,
            verify: false,
        };
        run_stop_hook(&hook, payload.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    fn activity_after(payload: &str, reported_exists: bool) -> String {
        let dir = tempfile::tempdir().unwrap();
        let reported = dir.path().join("reported");
        if reported_exists {
            touch(&reported);
        }
        let unreported = dir.path().join("unreported");
        let activity = dir.path().join("activity").join("A");
        let hook = StopHook {
            reported: &reported,
            unreported: &unreported,
            activity: Some(&activity),
            changes: None,
            usage: None,
            verify: false,
        };
        run_stop_hook(&hook, payload.as_bytes(), Vec::new()).unwrap();
        fs::read_to_string(activity).unwrap()
    }

    fn stop_row(verify: bool) -> serde_json::Value {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("s1.jsonl");
        fs::write(
            &transcript,
            r#"{"type":"assistant","message":{"id":"m1","model":"opus","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}}"#,
        )
        .unwrap();
        let target = UsageTarget {
            worker: "verify".into(),
            seen: dir.path().join("verify.usage-seen"),
            store: None,
            usage: dir.path().join("usage.jsonl"),
        };
        let unreported = dir.path().join("unreported");
        let hook = StopHook {
            reported: &dir.path().join("reported"),
            unreported: &unreported,
            activity: None,
            changes: None,
            usage: Some(&target),
            verify,
        };
        let payload = format!(
            r#"{{"session_id":"s1","transcript_path":"{}"}}"#,
            transcript.display()
        );
        run_stop_hook(&hook, payload.as_bytes(), Vec::new()).unwrap();
        let written = fs::read_to_string(&target.usage).unwrap();
        assert_eq!(written.lines().count(), 1);
        serde_json::from_str(written.trim()).unwrap()
    }

    #[test]
    fn a_verify_stop_hook_appends_one_verify_row_priced_like_a_worker_row() {
        let worker = stop_row(false);
        assert_eq!(worker["kind"], "worker");
        let row = stop_row(true);
        assert_eq!(row["kind"], "verify");
        assert_eq!(row["role"], "verify");
        assert_eq!(row["harness"], "claude-tmux");
        assert_eq!(row["cost_usd"], worker["cost_usd"]);
        if worker["cost_usd"].is_null() {
            assert!(row["cost_usd"].is_null());
        }
    }

    #[test]
    fn drop_worker_removes_its_queued_reports_and_logs_each() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        for (worker, seq) in [("A", 1), ("B", 1), ("A", 2)] {
            let report = Report {
                worker: worker.to_string(),
                at: format!("2026-11-10T13:00:0{seq}Z"),
                status: "done".to_string(),
                text: "t".to_string(),
            };
            inbox.post(&report, seq).unwrap();
        }
        assert_eq!(inbox.drop_worker("A").unwrap(), 2);
        let left: Vec<String> = inbox
            .peek()
            .unwrap()
            .into_iter()
            .map(|r| r.worker)
            .collect();
        assert_eq!(left, ["B"]);
        let log = fs::read_to_string(dir.path().join(DROP_LOG)).unwrap();
        assert_eq!(log.lines().count(), 2);
        assert_eq!(inbox.drop_worker("A").unwrap(), 0);
    }

    #[test]
    fn stop_hook_writes_background_while_tasks_run_and_idle_otherwise() {
        let busy = r#"{"stop_hook_active":false,"background_tasks":[{"id":"x"}]}"#;
        assert!(activity_after(busy, false).ends_with(" background"));
        let plain = r#"{"stop_hook_active":false}"#;
        assert!(activity_after(plain, true).ends_with(" idle"));
        assert!(activity_after("not json", false).ends_with(" idle"));
    }

    #[test]
    fn stop_hook_blocks_once_then_a_report_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let reported = dir.path().join("reported").join("A");
        let first = stop_hook(&reported, r#"{"stop_hook_active":false}"#);
        assert_eq!(
            serde_json::from_str::<Value>(&first).unwrap(),
            stop_block_output()
        );
        assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":true}"#), "");

        touch(&reported);
        assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":false}"#), "");
        assert!(!reported.exists());
    }

    #[test]
    fn stop_hook_give_up_writes_unreported_and_a_report_removes_both() {
        let dir = tempfile::tempdir().unwrap();
        let reported = dir.path().join("reported").join("A");
        let unreported = reported.with_file_name("unreported");
        assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":true}"#), "");
        assert!(unreported.exists());

        touch(&reported);
        assert_eq!(stop_hook(&reported, r#"{"stop_hook_active":true}"#), "");
        assert!(!reported.exists() && !unreported.exists());
    }

    #[test]
    fn stop_hook_with_background_tasks_leaves_markers_alone() {
        let dir = tempfile::tempdir().unwrap();
        let reported = dir.path().join("reported").join("A");
        let unreported = reported.with_file_name("unreported");
        touch(&reported);
        touch(&unreported);
        let busy = r#"{"stop_hook_active":false,"background_tasks":[{"id":"x"}]}"#;
        assert_eq!(stop_hook(&reported, busy), "");
        assert!(reported.exists() && unreported.exists());
    }

    #[test]
    fn stop_hook_lets_malformed_input_stop() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(stop_hook(&dir.path().join("A"), "not json"), "");
    }

    #[test]
    fn report_lands_in_inbox_and_drains_once() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"report","arguments":{"status":"done","text":"hello"}}}"#;
        let mut out = Vec::new();
        run_stdio_server(
            "GUZ-1",
            &inbox,
            None,
            format!("{call}\n").as_bytes(),
            &mut out,
        )
        .unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["result"]["isError"], false);
        assert!(dir.path().join("reported").join("GUZ-1").exists());

        assert_eq!(inbox.peek().unwrap().len(), 1);
        let reports = inbox.drain(Selection::All).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].worker, "GUZ-1");
        assert_eq!(reports[0].text, "hello");
        assert!(inbox.peek().unwrap().is_empty());
    }

    fn report_call(id: u32, text: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"report","arguments":{{"status":"done","text":"{text}"}}}}}}"#
        )
    }

    fn replies(inbox: &Inbox, calls: &[String]) -> Vec<Value> {
        let mut out = Vec::new();
        let input = format!("{}\n", calls.join("\n"));
        run_stdio_server("GUZ-1", inbox, None, input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn a_second_report_in_the_same_turn_is_refused_and_posts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox")).once_per_turn(true);
        let out = replies(&inbox, &[report_call(1, "first"), report_call(2, "second")]);
        assert_eq!(out[0]["result"]["isError"], false);
        assert_eq!(out[1]["result"]["isError"], true);
        assert!(out[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Already reported this turn"));
        let reports = inbox.drain(Selection::All).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].text, "first");
    }

    #[test]
    fn a_report_after_the_turn_consumed_the_marker_is_delivered() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox")).once_per_turn(true);
        replies(&inbox, &[report_call(1, "first")]);
        fs::remove_file(dir.path().join("reported").join("GUZ-1")).unwrap();
        let out = replies(&inbox, &[report_call(2, "next turn")]);
        assert_eq!(out[0]["result"]["isError"], false);
        assert_eq!(inbox.peek().unwrap().len(), 2);
    }

    #[test]
    fn without_the_once_per_turn_flag_repeat_reports_still_post() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let out = replies(&inbox, &[report_call(1, "a"), report_call(2, "b")]);
        assert_eq!(out[1]["result"]["isError"], false);
        assert_eq!(inbox.peek().unwrap().len(), 2);
    }

    fn status_call(id: u32, text: &str) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"set_status","arguments":{{"text":"{text}"}}}}}}"#
        )
    }

    #[test]
    fn set_status_appends_to_the_file_without_touching_the_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let input = format!(
            "{}\n{}\n",
            status_call(1, "first"),
            status_call(2, "second")
        );
        let mut out = Vec::new();
        run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
        let replies: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(replies.iter().all(|r| r["result"]["isError"] == false));
        assert_eq!(replies[1]["result"]["content"][0]["text"], "Status set.");

        let status = fs::read_to_string(status_path(dir.path(), "GUZ-1")).unwrap();
        let texts: Vec<_> = decode_statuses(&status)
            .into_iter()
            .map(|status| status.text)
            .collect();
        assert_eq!(texts, ["first", "second"]);
        let names: Vec<_> = fs::read_dir(dir.path().join("status"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["GUZ-1"]);
        assert!(inbox.peek().unwrap().is_empty());
        assert!(!dir.path().join("inbox").exists());
        assert!(!dir.path().join("reported").exists());
    }

    #[test]
    fn set_status_keeps_only_the_last_five() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let input: String = (1..=7)
            .map(|n| format!("{}\n", status_call(n, &format!("step {n}"))))
            .collect();
        let mut out = Vec::new();
        run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
        let status = fs::read_to_string(status_path(dir.path(), "GUZ-1")).unwrap();
        let texts: Vec<_> = decode_statuses(&status)
            .into_iter()
            .map(|status| status.text)
            .collect();
        assert_eq!(texts, ["step 3", "step 4", "step 5", "step 6", "step 7"]);
        assert_eq!(fs::read_dir(dir.path().join("status")).unwrap().count(), 1);
    }

    #[test]
    fn set_status_starts_from_an_old_single_entry_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = status_path(dir.path(), "GUZ-1");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "2026-01-01T00:00:00+00:00\nold\n").unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let input = format!("{}\n", status_call(1, "new"));
        let mut out = Vec::new();
        run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
        let texts: Vec<_> = decode_statuses(&fs::read_to_string(&path).unwrap())
            .into_iter()
            .map(|status| status.text)
            .collect();
        assert_eq!(texts, ["old", "new"]);
    }

    #[test]
    fn set_status_is_refused_after_close_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"))
            .requiring_owner(Some(dir.path().join("owner-gone")));
        let mut out = Vec::new();
        let input = format!("{}\n", status_call(1, "late"));
        run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "Could not set status: worker closed."
        );
        assert!(!dir.path().join("status").exists());
    }

    #[test]
    fn set_status_works_while_the_owner_exists() {
        let dir = tempfile::tempdir().unwrap();
        let owner = dir.path().join("owner");
        fs::write(&owner, "").unwrap();
        let inbox = Inbox::new(dir.path().join("inbox")).requiring_owner(Some(owner));
        let mut out = Vec::new();
        let input = format!("{}\n", status_call(1, "ok"));
        run_stdio_server("GUZ-1", &inbox, None, input.as_bytes(), &mut out).unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["result"]["isError"], false);
        assert!(status_path(dir.path(), "GUZ-1").exists());
    }

    #[test]
    fn drain_some_takes_only_that_position() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        for (seq, text) in [(1, "one"), (2, "two")] {
            let report = Report {
                worker: "A".to_string(),
                at: format!("2026-01-01T00:00:0{seq}Z"),
                status: "done".to_string(),
                text: text.to_string(),
            };
            inbox.post(&report, seq).unwrap();
        }
        let taken = inbox.drain(Selection::At("2".parse().unwrap())).unwrap();
        assert_eq!(taken[0].text, "two");
        assert_eq!(inbox.peek().unwrap().len(), 1);
    }

    #[test]
    fn drained_reports_leave_the_ledger_and_the_queue_restarts_at_one() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let post = |text: &str, seq| {
            let report = Report {
                worker: "A".to_string(),
                at: format!("2026-01-01T00:00:0{seq}Z"),
                status: "done".to_string(),
                text: text.to_string(),
            };
            inbox.post(&report, seq).unwrap();
        };
        post("one", 1);
        post("two", 2);
        assert_eq!(inbox.drain(Selection::All).unwrap().len(), 2);
        post("three", 3);

        let queued = inbox.peek().unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].text, "three");
        assert!(inbox.pending().unwrap().is_empty());
        let value: Value =
            serde_json::from_slice(&fs::read(dir.path().join("inbox.json")).unwrap()).unwrap();
        assert_eq!(value["entries"].as_array().unwrap().len(), 1);
    }

    fn same_name_report(text: &str) -> Report {
        Report {
            worker: "schedule:a".to_string(),
            at: "2026-01-01T00:00:00Z".to_string(),
            status: "blocked".to_string(),
            text: text.to_string(),
        }
    }

    fn queued_texts(inbox: &Inbox) -> Vec<String> {
        inbox.peek().unwrap().into_iter().map(|r| r.text).collect()
    }

    #[test]
    fn two_posts_with_one_file_name_both_reach_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        inbox.post(&same_name_report("one"), 0).unwrap();
        inbox.post(&same_name_report("two"), 0).unwrap();
        assert_eq!(queued_texts(&inbox), ["one", "two"]);
        assert_eq!(fs::read_dir(dir.path().join("inbox")).unwrap().count(), 0);
    }

    #[test]
    fn a_post_that_reuses_a_queued_file_name_reaches_the_queue() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        inbox.post(&same_name_report("one"), 0).unwrap();
        assert_eq!(queued_texts(&inbox), ["one"]);
        inbox.post(&same_name_report("two"), 0).unwrap();
        assert_eq!(queued_texts(&inbox), ["one", "two"]);
    }

    #[test]
    fn a_report_removes_the_unreported_marker() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let unreported = unreported_path(dir.path(), "A");
        touch(&unreported);
        inbox.mark_reported("A").unwrap();
        assert!(!unreported.exists());
        assert!(reported_path(dir.path(), "A").exists());
    }

    #[test]
    fn a_report_removes_the_sent_marker() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let sent = sent_path(dir.path(), "A");
        touch(&sent);
        inbox.mark_reported("A").unwrap();
        assert!(!sent.exists());
    }
}

#[cfg(test)]
#[path = "worker_mcp_tests.rs"]
mod inbox_tests;
