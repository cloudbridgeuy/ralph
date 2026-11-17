use super::{read_meta, thread_dir, write_meta};
use crate::chat::session::store_dir;
use crate::chat::ChatExit;
use ralph_core::chat_thread::{ThreadMeta, ThreadName};
use ralph_core::chat_thread_auto::{close_plan, thread_closed_report, CloseEvent, ClosePlan};
use ralph_core::worker::INBOX_DIR;
use ralph_server::worker_mcp::Inbox;
use std::path::{Path, PathBuf};

pub(crate) fn record_exit(store: Option<PathBuf>, thread: Option<&ThreadName>, exit: &ChatExit) {
    let Some(name) = thread else { return };
    let event = match exit {
        ChatExit::Closed => CloseEvent::Closed,
        ChatExit::Switch(..) => CloseEvent::Switched,
        ChatExit::Quit => CloseEvent::Other,
    };
    let at = chrono::Utc::now().to_rfc3339();
    let result = store_dir(store)
        .map_err(|e| e.to_string())
        .and_then(|main| record_close(&main, name, event, &at));
    if let Err(e) = result {
        eprintln!("Could not record the close of thread {name}: {e}");
    }
}

pub(super) fn record_close(
    main: &Path,
    name: &ThreadName,
    event: CloseEvent,
    at: &str,
) -> Result<(), String> {
    let dir = thread_dir(main, name);
    let meta = read_meta(&dir).map_err(|e| e.to_string())?;
    let plan = close_plan(event, &meta);
    if plan == ClosePlan::Nothing {
        return Ok(());
    }
    let marked = write_meta(
        &dir,
        &ThreadMeta {
            closed: true,
            ..meta.clone()
        },
    )
    .map_err(|e| format!("could not mark it closed: {e}"));
    let posted = match (plan, meta.auto) {
        (ClosePlan::MarkAndReport, Some(auto)) => Inbox::new(main.join(INBOX_DIR))
            .post(&thread_closed_report(name, auto, at), 0)
            .map_err(|e| format!("could not post the report: {e}")),
        _ => Ok(()),
    };
    marked.and(posted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_thread::AutoMode;
    use ralph_core::chat_thread_auto::THREAD_CLOSED_STATUS;
    use ralph_core::worker::Report;
    use std::fs;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn thread_with(main: &Path, n: &str, auto: Option<AutoMode>) {
        let dir = thread_dir(main, &name(n));
        fs::create_dir_all(&dir).unwrap();
        let meta = ThreadMeta {
            base: 0,
            created: "t".to_string(),
            merged: None,
            parked: None,
            auto,
            closed: false,
        };
        write_meta(&dir, &meta).unwrap();
    }

    fn closed(main: &Path, n: &str) -> bool {
        read_meta(&thread_dir(main, &name(n))).unwrap().closed
    }

    fn posted(main: &Path) -> Vec<Report> {
        let Ok(entries) = fs::read_dir(main.join(INBOX_DIR)) else {
            return Vec::new();
        };
        entries
            .map(|e| serde_json::from_slice(&fs::read(e.unwrap().path()).unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn close_marks_and_leaves_an_unread_report_for_main() {
        let main = tempfile::tempdir().unwrap();
        thread_with(main.path(), "side", Some(AutoMode::Merge));
        record_close(
            main.path(),
            &name("side"),
            CloseEvent::Closed,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        assert!(closed(main.path(), "side"));
        let reports = posted(main.path());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, THREAD_CLOSED_STATUS);
        assert_eq!(
            reports[0].text,
            "side closed \u{b7} Enter to merge or keep it"
        );
        assert!(!main.path().join(INBOX_DIR).join("inbox.json").exists());
    }

    #[test]
    fn switch_marks_without_a_report() {
        let main = tempfile::tempdir().unwrap();
        thread_with(main.path(), "side", Some(AutoMode::Drop));
        record_close(main.path(), &name("side"), CloseEvent::Switched, "t").unwrap();
        assert!(closed(main.path(), "side"));
        assert!(posted(main.path()).is_empty());
    }

    #[test]
    fn other_exits_and_threads_without_auto_change_nothing() {
        let main = tempfile::tempdir().unwrap();
        thread_with(main.path(), "side", Some(AutoMode::Merge));
        thread_with(main.path(), "plain", None);
        record_close(main.path(), &name("side"), CloseEvent::Other, "t").unwrap();
        record_close(main.path(), &name("plain"), CloseEvent::Closed, "t").unwrap();
        assert!(!closed(main.path(), "side"));
        assert!(!closed(main.path(), "plain"));
        assert!(posted(main.path()).is_empty());
    }

    #[test]
    fn two_threads_write_two_reports_and_an_old_thread_json_is_read() {
        let main = tempfile::tempdir().unwrap();
        thread_with(main.path(), "one", Some(AutoMode::Merge));
        thread_with(main.path(), "two", Some(AutoMode::Drop));
        for n in ["one", "two"] {
            record_close(
                main.path(),
                &name(n),
                CloseEvent::Closed,
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        }
        assert_eq!(posted(main.path()).len(), 2);
        let old = thread_dir(main.path(), &name("old"));
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("thread.json"), r#"{"base":0,"created":"t"}"#).unwrap();
        record_close(main.path(), &name("old"), CloseEvent::Closed, "t").unwrap();
        assert!(!closed(main.path(), "old"));
    }
}
