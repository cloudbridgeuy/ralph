use ralph_core::inbox::{
    decode_message, drop_log_line, DecodeError, Entry, Ledger, Position, Selection,
};
use ralph_core::mcp::tool_result;
use ralph_core::worker::WorkerId;
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
const POST_ATTEMPTS: u64 = 1000;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedgerChange {
    Changed,
    Unchanged,
}

impl LedgerChange {
    fn from_changed(changed: bool) -> Self {
        if changed {
            Self::Changed
        } else {
            Self::Unchanged
        }
    }
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
        for next in seq..seq.saturating_add(POST_ATTEMPTS) {
            match file.persist_noclobber(self.dir.join(inbox_file_name(report, next))) {
                Ok(_) => return Ok(()),
                Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => file = e.file,
                Err(e) => return Err(e.error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("no free inbox file name after {POST_ATTEMPTS} tries"),
        ))
    }

    fn reported_this_turn(&self, worker: &str) -> bool {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        refuses_repeat_report(self.once_per_turn, reported_path(store, worker).exists())
    }

    fn log_report(&self, report: &Report, status: ReportStatus, seq: u64) {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        outcome_log::record(store, &report.from.to_string(), |key| {
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
        act: impl FnOnce(&mut Ledger) -> (T, LedgerChange),
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
        let (value, change) = act(&mut ledger);
        let should_save = match (state, change) {
            (LedgerState::Repaired, LedgerChange::Changed | LedgerChange::Unchanged)
            | (LedgerState::Unchanged, LedgerChange::Changed) => true,
            (LedgerState::Unchanged, LedgerChange::Unchanged) => ingested,
        };
        if should_save {
            self.save_ledger(&ledger)?;
        }
        for path in accepted {
            remove_if_present(&path)?;
        }
        Ok(value)
    }

    fn sync<T>(&self, act: impl FnOnce(&mut Ledger) -> (T, LedgerChange)) -> io::Result<T> {
        self.sync_with(&mut |message| crate::warn::warn(message), act)
    }

    pub fn peek(&self) -> io::Result<Vec<Report>> {
        self.sync(|ledger| (ledger.reports(), LedgerChange::Unchanged))
    }

    pub fn unread_count(&self) -> io::Result<usize> {
        Ok(self.load_ledger()?.reports().len() + self.pending()?.len())
    }

    pub fn drain(&self, selection: Selection) -> io::Result<Vec<Report>> {
        self.sync(|ledger| {
            let reports = ledger.take(selection);
            let change = LedgerChange::from_changed(!reports.is_empty());
            (reports, change)
        })
    }

    pub fn drain_closes(&self) -> io::Result<Vec<Report>> {
        self.sync(|ledger| {
            let reports = ledger.take_closes();
            let change = LedgerChange::from_changed(!reports.is_empty());
            (reports, change)
        })
    }

    pub fn drop_report(&self, position: Position) -> io::Result<Option<Entry>> {
        let log = self.dir.with_file_name(DROP_LOG);
        self.sync(|ledger| match ledger.get(position).cloned() {
            None => (Ok(None), LedgerChange::Unchanged),
            Some(entry) => match append_line(&log, &drop_log_line(&entry, chrono::Utc::now())) {
                Err(e) => (Err(e), LedgerChange::Unchanged),
                Ok(()) => {
                    ledger.remove(position);
                    (Ok(Some(entry)), LedgerChange::Changed)
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
            let change = LedgerChange::from_changed(!dropped.is_empty());
            (logged.map(|()| dropped.len()), change)
        })?
    }

    pub fn ledger(&self) -> io::Result<Ledger> {
        self.sync(|ledger| (ledger.clone(), LedgerChange::Unchanged))
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
    let worker_id = WorkerId::parse(worker).map_err(io::Error::other)?;
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
                let report = Report::from_worker(
                    &worker_id,
                    &chrono::Utc::now().to_rfc3339(),
                    status,
                    changes.map_or(text.clone(), |changes| changes.attach_to(&text)),
                );
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
mod tests;

#[cfg(test)]
#[path = "worker_mcp_tests.rs"]
mod inbox_tests;
