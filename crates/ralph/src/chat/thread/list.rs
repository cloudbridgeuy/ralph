//! The thread listing: gathers a row per thread and prints the table.

use super::{read_meta, thread_dir};
use crate::chat::inbox::Display;
use crate::chat::prompt::screen_columns;
use crate::chat::workers::shown;
use crate::highlight::Highlighter;
use crate::markdown::MarkdownTheme;
use crate::render::RenderContext;
use crate::ui_theme::UiColors;
use ralph_core::chat_thread::{ThreadName, THREADS_DIR};
use ralph_core::chat_thread_table::{
    sort_rows, threads_view, ThreadRow, ThreadState, ThreadTableOpts,
};
use ralph_core::worker::INBOX_DIR;
use ralph_server::chat::memory::Store;
use ralph_server::worker_mcp::Inbox;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::Path;

pub fn rows(main: &Path, workers: &dyn Fn(&ThreadName) -> usize) -> io::Result<Vec<ThreadRow>> {
    let entries = match fs::read_dir(main.join(THREADS_DIR)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries?,
    };
    let mut rows = Vec::new();
    for name in
        entries.filter_map(|entry| ThreadName::parse(entry.ok()?.file_name().to_str()?).ok())
    {
        let dir = thread_dir(main, &name);
        let Ok(meta) = read_meta(&dir) else {
            continue;
        };
        let state = match (meta.merged, meta.parked) {
            (Some(merged), _) => ThreadState::Merged(merged.at),
            (None, Some(parked)) => ThreadState::Parked(parked.at),
            (None, None) => ThreadState::Open,
        };
        rows.push(ThreadRow {
            state,
            base: meta.base,
            created: meta.created,
            active: log_mtime(&dir),
            messages: Store::new(dir.clone()).log_len()?,
            workers: workers(&name),
            unread: Inbox::new(dir.join(INBOX_DIR)).unread_count()?,
            name: name.to_string(),
        });
    }
    sort_rows(&mut rows);
    Ok(rows)
}

pub(in crate::chat) fn listing(rows: &[ThreadRow], display: &Display, empty: &str) -> String {
    let opts = ThreadTableOpts {
        now: chrono::Utc::now(),
        width: screen_columns(),
        styled: display.color,
    };
    match threads_view(rows, &opts) {
        Some(view) => shown(&view, display).trim_end_matches('\n').to_string(),
        None => empty.to_string(),
    }
}

pub(in crate::chat) fn default_listing(rows: &[ThreadRow], empty: &str) -> String {
    let color = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let highlighter = Highlighter::new();
    let theme = MarkdownTheme::default();
    let display = Display {
        ctx: RenderContext::new(&highlighter, color),
        theme: &theme,
        color,
        ui: UiColors::default(),
    };
    listing(rows, &display, empty)
}

fn log_mtime(dir: &Path) -> Option<String> {
    let modified = fs::metadata(dir.join("log.jsonl")).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::thread::tests::{forked, name};
    use crate::chat::thread::{ensure, park_thread};
    use ralph_core::chat_thread::merged_report;

    #[test]
    fn rows_gather_state_counts_and_the_log_mtime() {
        let (_root, main, dir) = forked();
        let auth = name("auth");
        ensure(&main, &name("b-side")).unwrap();
        Inbox::new(dir.join(INBOX_DIR))
            .post(&merged_report(&auth, "a", "t1"), 0)
            .unwrap();
        park_thread(&main, &name("b-side"), &[]).unwrap();
        let rows = rows(&main, &|n| usize::from(n.as_str() == "auth") * 3).unwrap();
        assert_eq!(rows.len(), 2);
        let (auth_row, side) = (&rows[0], &rows[1]);
        assert_eq!(auth_row.name, "auth");
        assert_eq!(auth_row.state, ThreadState::Open);
        assert_eq!((auth_row.base, auth_row.messages), (2, 1));
        assert_eq!((auth_row.workers, auth_row.unread), (3, 1));
        let modified = fs::metadata(dir.join("log.jsonl"))
            .unwrap()
            .modified()
            .unwrap();
        let expected = chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339();
        assert_eq!(auth_row.active, Some(expected));
        assert_eq!(side.name, "b-side");
        assert!(matches!(side.state, ThreadState::Parked(_)));
        assert_eq!(side.active, None);
    }

    #[test]
    fn listing_prints_a_table_or_the_empty_message() {
        let (_root, main, _dir) = forked();
        let highlighter = Highlighter::new();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::new(&highlighter, false),
            theme: &theme,
            color: false,
            ui: UiColors::default(),
        };
        let listed = rows(&main, &|_| 0).unwrap();
        let text = listing(&listed, &display, "none");
        assert!(text.starts_with("| NAME | STATE | FORK |"), "{text}");
        assert!(text.contains("| auth | open | #2 |"), "{text}");
        assert!(!text.ends_with('\n'));
        assert_eq!(listing(&[], &display, "none"), "none");
    }
}
