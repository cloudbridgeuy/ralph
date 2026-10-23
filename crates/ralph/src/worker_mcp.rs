use ralph_core::inbox::{drop_log_line, Entry, Ledger, Position, Selection};
use ralph_core::mcp::tool_result;
use ralph_core::worker::{
    accept_report, append_status, decode_statuses, encode_statuses, handle_request,
    inbox_file_name, reported_path, sent_path, status_path, stop_block_output, stop_decision,
    unreported_path, Outcome, Report, StatusLine, StopDecision,
};
use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::worker_changes::WorkerChanges;

const DROP_LOG: &str = "inbox-drops.jsonl";

pub struct Inbox {
    dir: PathBuf,
    owner: Option<PathBuf>,
    lock: Mutex<()>,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            owner: None,
            lock: Mutex::new(()),
        }
    }

    pub fn requiring_owner(mut self, owner: Option<PathBuf>) -> Self {
        self.owner = owner;
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
        let name = inbox_file_name(report, seq);
        let tmp = self.dir.join(format!(".{name}"));
        fs::write(&tmp, serde_json::to_vec(report)?)?;
        fs::rename(tmp, self.dir.join(name))
    }

    fn set_status(&self, worker: &str, text: &str) -> io::Result<()> {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        let path = status_path(store, worker);
        let dir = path.parent().unwrap_or(store);
        fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{worker}.tmp"));
        let history = fs::read_to_string(&path)
            .map(|raw| decode_statuses(&raw))
            .unwrap_or_default();
        let status = StatusLine {
            at: chrono::Utc::now().to_rfc3339(),
            text: text.to_string(),
        };
        fs::write(&tmp, encode_statuses(&append_status(history, status)))?;
        fs::rename(tmp, path)
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
                .filter_map(|e| e.ok().map(|e| e.path()))
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

    fn load_ledger(&self) -> io::Result<Ledger> {
        match fs::read(self.ledger_path()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Ledger::default()),
            Err(e) => Err(e),
        }
    }

    fn save_ledger(&self, ledger: &Ledger) -> io::Result<()> {
        let path = self.ledger_path();
        let tmp = path.with_file_name(".inbox.json.tmp");
        fs::write(&tmp, serde_json::to_vec(ledger)?)?;
        fs::rename(tmp, path)
    }

    fn sync<T>(&self, act: impl FnOnce(&mut Ledger) -> (T, bool)) -> io::Result<T> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut ledger = self.load_ledger()?;
        let pending = self.pending()?;
        let mut arrivals = Vec::new();
        for path in &pending {
            match serde_json::from_slice(&fs::read(path)?) {
                Ok(report) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    arrivals.push((name, report));
                }
                Err(e) => crate::warn::warn(format!(
                    "dropping malformed inbox file {}: {e}",
                    path.display()
                )),
            }
        }
        ledger.ingest(arrivals);
        let (value, changed) = act(&mut ledger);
        if !pending.is_empty() || changed {
            self.save_ledger(&ledger)?;
        }
        for path in pending {
            remove_if_present(&path)?;
        }
        Ok(value)
    }

    pub fn peek(&self) -> io::Result<Vec<Report>> {
        self.sync(|ledger| (ledger.reports(), false))
    }

    pub fn drain(&self, selection: Selection) -> io::Result<Vec<Report>> {
        self.sync(|ledger| {
            let reports = ledger.take(selection);
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
    path.extension().is_some_and(|ext| ext == "json")
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
                seq += 1;
                let report = Report {
                    worker: worker.to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                    status: status.as_str().to_string(),
                    text: changes.map_or(text.clone(), |changes| changes.attach_to(&text)),
                };
                match inbox.post(&report, seq) {
                    Ok(()) => {
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

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

pub fn run_stop_hook(
    reported: &Path,
    unreported: &Path,
    changes: Option<&WorkerChanges>,
    mut input: impl Read,
    mut output: impl Write,
) -> io::Result<()> {
    let mut payload = String::new();
    if let Err(e) = input.read_to_string(&mut payload) {
        crate::warn::warn(format!(
            "letting the worker stop: unreadable Stop hook input: {e}"
        ));
        return Ok(());
    }
    if let Some(changes) = changes {
        changes.snapshot_turn_or_warn();
    }
    match stop_decision(&payload, reported.exists()) {
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
        run_stop_hook(reported, &unreported, None, payload.as_bytes(), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
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
