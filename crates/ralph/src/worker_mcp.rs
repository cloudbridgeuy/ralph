use ralph_core::mcp::tool_result;
use ralph_core::worker::{handle_request, inbox_file_name, Outcome, Report};
use serde_json::Value;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

pub struct Inbox {
    dir: PathBuf,
}

impl Inbox {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn post(&self, report: &Report, seq: u64) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let name = inbox_file_name(report, seq);
        let tmp = self.dir.join(format!(".{name}"));
        fs::write(&tmp, serde_json::to_vec(report)?)?;
        fs::rename(tmp, self.dir.join(name))
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

    pub fn peek(&self) -> io::Result<Vec<Report>> {
        Ok(self
            .pending()?
            .iter()
            .filter_map(|path| serde_json::from_slice(&fs::read(path).ok()?).ok())
            .collect())
    }

    pub fn drain(&self) -> io::Result<Vec<Report>> {
        let mut reports = Vec::new();
        for path in self.pending()? {
            match serde_json::from_slice(&fs::read(&path)?) {
                Ok(report) => reports.push(report),
                Err(e) => crate::warn::warn(format!(
                    "dropping malformed inbox file {}: {e}",
                    path.display()
                )),
            }
            fs::remove_file(path)?;
        }
        Ok(reports)
    }
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
                    status,
                    text,
                };
                match inbox.post(&report, seq) {
                    Ok(()) => tool_result(
                        id,
                        "Delivered. Stop and wait for the person's answer.",
                        false,
                    ),
                    Err(e) => tool_result(id, &format!("Could not deliver: {e}"), true),
                }
            }
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lands_in_inbox_and_drains_once() {
        let dir = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(dir.path().join("inbox"));
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"report","arguments":{"status":"done","text":"hello"}}}"#;
        let mut out = Vec::new();
        run_stdio_server("GUZ-1", &inbox, format!("{call}\n").as_bytes(), &mut out).unwrap();
        let reply: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(reply["result"]["isError"], false);

        assert_eq!(inbox.peek().unwrap().len(), 1);
        let reports = inbox.drain().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].worker, "GUZ-1");
        assert_eq!(reports[0].text, "hello");
        assert!(inbox.peek().unwrap().is_empty());
    }
}
