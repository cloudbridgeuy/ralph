use crate::cli::{InboxArgs, InboxCommand};
use crate::worker_mcp::Inbox;
use ralph_core::inbox_post::{check_store, external_report, MAX_TEXT_BYTES};
use ralph_core::worker::INBOX_DIR;
use std::error::Error;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const STDIN_MARK: &str = "-";

pub fn run(args: InboxArgs) -> Result<(), Box<dyn Error>> {
    match args.command {
        InboxCommand::Post { store, from, input } => {
            let store = std::env::current_dir()?.join(store);
            let at = chrono::Utc::now().to_rfc3339();
            post(&store, &from, &input, &mut io::stdin().lock(), &at)?;
            println!(
                "Posted a report from external:{from} to {}",
                store.display()
            );
            Ok(())
        }
    }
}

fn read_bounded(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_TEXT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_input(input: &Path, stdin: &mut impl Read) -> Result<Vec<u8>, String> {
    if input == Path::new(STDIN_MARK) {
        return read_bounded(stdin).map_err(|e| format!("Could not read stdin: {e}"));
    }
    let mut file =
        File::open(input).map_err(|e| format!("Could not open {}: {e}", input.display()))?;
    read_bounded(&mut file).map_err(|e| format!("Could not read {}: {e}", input.display()))
}

pub fn post(
    store: &Path,
    source: &str,
    input: &Path,
    stdin: &mut impl Read,
    at: &str,
) -> Result<(), String> {
    check_store(
        store.is_dir(),
        store.join("log.jsonl").exists(),
        store.join("LOCK").exists(),
    )?;
    let bytes = read_input(input, stdin)?;
    let report = external_report(source, at, &bytes)?;
    Inbox::new(store.join(INBOX_DIR))
        .post(&report, 0)
        .map_err(|e| format!("Could not write the report: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::worker::Report;
    use std::fs;

    fn store() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("log.jsonl"), "").unwrap();
        dir
    }

    fn posted(store: &Path) -> Vec<Report> {
        let Ok(entries) = fs::read_dir(store.join(INBOX_DIR)) else {
            return Vec::new();
        };
        entries
            .map(|e| serde_json::from_slice(&fs::read(e.unwrap().path()).unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn read_bounded_stops_one_byte_past_the_cap() {
        let big = vec![b'a'; MAX_TEXT_BYTES * 2];
        assert_eq!(
            read_bounded(&mut big.as_slice()).unwrap().len(),
            MAX_TEXT_BYTES + 1
        );
        assert_eq!(read_bounded(&mut &b"abc"[..]).unwrap(), b"abc");
    }

    #[test]
    fn posts_stdin_text_into_the_inbox() {
        let dir = store();
        post(
            dir.path(),
            "ci",
            Path::new("-"),
            &mut &b"all green"[..],
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let reports = posted(dir.path());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].worker, "external:ci");
        assert_eq!(reports[0].status, "report");
        assert_eq!(reports[0].text, "all green");
    }

    #[test]
    fn posts_a_file_by_path() {
        let dir = store();
        let file = dir.path().join("note.txt");
        fs::write(&file, "from a file").unwrap();
        post(
            dir.path(),
            "ci",
            &file,
            &mut io::empty(),
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(posted(dir.path())[0].text, "from a file");
    }

    #[test]
    fn a_missing_input_file_is_an_error_and_writes_nothing() {
        let dir = store();
        let err = post(
            dir.path(),
            "ci",
            &dir.path().join("none.txt"),
            &mut io::empty(),
            "t",
        )
        .unwrap_err();
        assert!(err.contains("Could not open"), "{err}");
        assert!(posted(dir.path()).is_empty());
    }

    #[test]
    fn an_oversized_report_is_refused_and_writes_nothing() {
        let dir = store();
        let big = vec![b'a'; MAX_TEXT_BYTES + 1];
        let err = post(dir.path(), "ci", Path::new("-"), &mut big.as_slice(), "t").unwrap_err();
        assert!(err.contains("65536"), "{err}");
        assert!(posted(dir.path()).is_empty());
    }

    #[test]
    fn a_folder_that_is_not_a_store_is_refused_and_not_touched() {
        let dir = tempfile::tempdir().unwrap();
        let err = post(dir.path(), "ci", Path::new("-"), &mut &b"x"[..], "t").unwrap_err();
        assert!(err.contains("not a chat store"), "{err}");
        assert!(!dir.path().join(INBOX_DIR).exists());
    }

    #[test]
    fn a_missing_store_is_refused_and_not_created() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(post(&missing, "ci", Path::new("-"), &mut &b"x"[..], "t").is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn a_store_with_only_a_lock_file_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("LOCK"), "").unwrap();
        post(dir.path(), "ci", Path::new("-"), &mut &b"x"[..], "t").unwrap();
        assert_eq!(posted(dir.path()).len(), 1);
    }

    #[test]
    fn a_report_with_control_characters_is_refused_and_writes_nothing() {
        let dir = store();
        let err = post(
            dir.path(),
            "ci",
            Path::new("-"),
            &mut &b"\x1b[2Jhi"[..],
            "t",
        )
        .unwrap_err();
        assert!(err.contains("control characters"), "{err}");
        assert!(posted(dir.path()).is_empty());
    }
}
