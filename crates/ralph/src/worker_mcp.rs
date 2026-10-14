use ralph_core::inbox::{drop_log_line, Entry, Ledger, ReportId};
use ralph_core::mcp::tool_result;
use ralph_core::worker::{
    handle_request, inbox_file_name, reported_path, stop_block_output, stop_decision,
    unreported_path, Outcome, Report, StopDecision,
};
use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const DROP_LOG: &str = "inbox-drops.jsonl";

pub struct Inbox {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }

    pub fn post(&self, report: &Report, seq: u64) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let name = inbox_file_name(report, seq);
        let tmp = self.dir.join(format!(".{name}"));
        fs::write(&tmp, serde_json::to_vec(report)?)?;
        fs::rename(tmp, self.dir.join(name))
    }

    fn mark_reported(&self, worker: &str) -> io::Result<()> {
        let store = self.dir.parent().unwrap_or(Path::new("."));
        let marker = reported_path(store, worker);
        if let Some(dir) = marker.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(marker, "")?;
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
        self.sync(|ledger| (ledger.unread(), false))
    }

    pub fn drain(&self) -> io::Result<Vec<Report>> {
        self.sync(|ledger| {
            let reports = ledger.read_unread();
            let changed = !reports.is_empty();
            (reports, changed)
        })
    }

    pub fn drop_report(&self, id: ReportId) -> io::Result<Option<Entry>> {
        let log = self.dir.with_file_name(DROP_LOG);
        self.sync(|ledger| match ledger.get(id).cloned() {
            None => (Ok(None), false),
            Some(entry) => match append_line(&log, &drop_log_line(&entry, chrono::Utc::now())) {
                Err(e) => (Err(e), false),
                Ok(()) => {
                    ledger.remove(id);
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
            Outcome::Report { id, status, text } => {
                seq += 1;
                let report = Report {
                    worker: worker.to_string(),
                    at: chrono::Utc::now().to_rfc3339(),
                    status: status.as_str().to_string(),
                    text,
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
        run_stop_hook(reported, &unreported, payload.as_bytes(), &mut out).unwrap();
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
        run_stdio_server("GUZ-1", &inbox, format!("{call}\n").as_bytes(), &mut out).unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["result"]["isError"], false);
        assert!(dir.path().join("reported").join("GUZ-1").exists());

        assert_eq!(inbox.peek().unwrap().len(), 1);
        let reports = inbox.drain().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].worker, "GUZ-1");
        assert_eq!(reports[0].text, "hello");
        assert!(inbox.peek().unwrap().is_empty());
    }

    #[test]
    fn ledger_keeps_stable_ids_across_reads() {
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
        assert_eq!(inbox.drain().unwrap().len(), 2);
        post("three", 3);

        let unread = inbox.peek().unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].text, "three");
        let value: Value =
            serde_json::from_slice(&fs::read(dir.path().join("inbox.json")).unwrap()).unwrap();
        let summary: Vec<_> = value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["id"].as_u64().unwrap(), e["read"].as_bool().unwrap()))
            .collect();
        assert_eq!(summary, [(1, true), (2, true), (3, false)]);
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
}
