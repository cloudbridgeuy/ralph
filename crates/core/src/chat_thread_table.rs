//! `ralph thread ls` and `/threads`: the thread rows and their table, built
//! from data the shell collected.

use crate::inbox::age;
use crate::workers_view::{block, dim, markdown, plain, span, table_width, Span, Tone, View};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadState {
    Open,
    Parked(String),
    Merged(String),
}

impl ThreadState {
    fn rank(&self) -> u8 {
        match self {
            ThreadState::Open => 0,
            ThreadState::Parked(_) => 1,
            ThreadState::Merged(_) => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    pub name: String,
    pub state: ThreadState,
    pub base: usize,
    pub created: String,
    pub active: Option<String>,
    pub messages: usize,
    pub workers: usize,
    pub unread: usize,
}

pub fn sort_rows(rows: &mut [ThreadRow]) {
    rows.sort_by(|a, b| (a.state.rank(), &a.name).cmp(&(b.state.rank(), &b.name)));
}

#[derive(Debug, Clone, Copy)]
pub struct ThreadTableOpts {
    pub now: DateTime<Utc>,
    pub width: usize,
    pub styled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Name,
    State,
    Fork,
    Created,
    Active,
    Messages,
    Workers,
    Unread,
}

const ALL_COLUMNS: [Column; 8] = [
    Column::Name,
    Column::State,
    Column::Fork,
    Column::Created,
    Column::Active,
    Column::Messages,
    Column::Workers,
    Column::Unread,
];
const DROP_ORDER: [Column; 3] = [Column::Active, Column::Unread, Column::Workers];

impl Column {
    fn header(self) -> &'static str {
        match self {
            Column::Name => "NAME",
            Column::State => "STATE",
            Column::Fork => "FORK",
            Column::Created => "CREATED",
            Column::Active => "ACTIVE",
            Column::Messages => "MSGS",
            Column::Workers => "WORKERS",
            Column::Unread => "UNREAD",
        }
    }

    fn cell(self, row: &ThreadRow, now: DateTime<Utc>) -> Vec<Span> {
        let ago = |at: &str| ago(at, now);
        let count = |n: usize, tone: Tone| match n {
            0 => dim("-"),
            n => span(tone, n.to_string()),
        };
        vec![match self {
            Column::Name => span(Tone::Name, row.name.as_str()),
            Column::State => state_span(&row.state, now),
            Column::Fork => plain(format!("#{}", row.base)),
            Column::Created => time_span(ago(&row.created)),
            Column::Active => row
                .active
                .as_deref()
                .map_or_else(|| dim("-"), |at| time_span(ago(at))),
            Column::Messages => plain(row.messages.to_string()),
            Column::Workers => count(row.workers, Tone::Active),
            Column::Unread => count(row.unread, Tone::Attention),
        }]
    }
}

fn ago(at: &str, now: DateTime<Utc>) -> Option<String> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|_| format!("{} ago", age(at, now)))
}

fn time_span(ago: Option<String>) -> Span {
    ago.map_or_else(|| dim("-"), plain)
}

fn state_span(state: &ThreadState, now: DateTime<Utc>) -> Span {
    match state {
        ThreadState::Open => span(Tone::Done, "open"),
        ThreadState::Parked(at) => span(
            Tone::Attention,
            format!("parked {}", ago(at, now).unwrap_or_else(|| "-".into())),
        ),
        ThreadState::Merged(at) => dim(format!(
            "merged {}",
            ago(at, now).unwrap_or_else(|| "-".into())
        )),
    }
}

fn table(rows: &[ThreadRow], columns: &[Column], opts: &ThreadTableOpts) -> Option<String> {
    let headers: Vec<&str> = columns.iter().map(|c| c.header()).collect();
    let cells: Vec<Vec<Vec<Span>>> = rows
        .iter()
        .map(|row| columns.iter().map(|c| c.cell(row, opts.now)).collect())
        .collect();
    (table_width(&headers, &cells) <= opts.width).then(|| markdown(&headers, &cells, opts.styled))
}

fn blocks(rows: &[ThreadRow], opts: &ThreadTableOpts) -> Vec<Vec<Span>> {
    rows.iter()
        .map(|row| {
            let state = state_span(&row.state, opts.now);
            let title = vec![
                span(state.tone, "●"),
                plain(" "),
                span(Tone::Name, row.name.as_str()),
                plain(" · "),
                state,
            ];
            let labels: Vec<String> = ALL_COLUMNS[2..]
                .iter()
                .map(|c| c.header().to_lowercase())
                .collect();
            let lines: Vec<(&str, Vec<Span>)> = ALL_COLUMNS[2..]
                .iter()
                .zip(&labels)
                .map(|(c, label)| (label.as_str(), c.cell(row, opts.now)))
                .collect();
            block(title, &lines, opts.width)
        })
        .collect::<Vec<_>>()
        .join(&Vec::new())
}

pub fn threads_view(rows: &[ThreadRow], opts: &ThreadTableOpts) -> Option<View> {
    if rows.is_empty() {
        return None;
    }
    let mut columns = ALL_COLUMNS.to_vec();
    loop {
        if let Some(markdown) = table(rows, &columns, opts) {
            return Some(View::Table(markdown));
        }
        let Some(next) = DROP_ORDER.iter().find(|c| columns.contains(c)) else {
            return Some(View::Text(blocks(rows, opts)));
        };
        columns.retain(|c| c != next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workers_view::{lines_text, TONE_DONE, TONE_END};

    pub(super) fn row(name: &str, state: ThreadState) -> ThreadRow {
        ThreadRow {
            name: name.into(),
            state,
            base: 1,
            created: "2026-01-01T00:00:00Z".into(),
            active: None,
            messages: 0,
            workers: 0,
            unread: 0,
        }
    }

    #[test]
    fn rows_sort_open_then_parked_then_merged_then_name() {
        let at = || "2026-01-02T00:00:00Z".to_string();
        let mut rows = vec![
            row("a-merged", ThreadState::Merged(at())),
            row("z-open", ThreadState::Open),
            row("b-parked", ThreadState::Parked(at())),
            row("a-open", ThreadState::Open),
            row("a-parked", ThreadState::Parked(at())),
        ];
        sort_rows(&mut rows);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["a-open", "z-open", "a-parked", "b-parked", "a-merged"]
        );
    }

    fn opts(width: usize, styled: bool) -> ThreadTableOpts {
        ThreadTableOpts {
            now: "2026-01-10T00:00:00Z".parse().unwrap(),
            width,
            styled,
        }
    }

    fn sample() -> Vec<ThreadRow> {
        let mut api = row("api-design", ThreadState::Open);
        api.base = 12;
        api.created = "2026-01-08T00:00:00Z".into();
        api.active = Some("2026-01-09T21:00:00Z".into());
        api.messages = 14;
        api.workers = 1;
        api.unread = 2;
        let mut old = row(
            "old-idea",
            ThreadState::Parked("2026-01-03T00:00:00Z".into()),
        );
        old.base = 5;
        old.created = "2026-01-01T00:00:00Z".into();
        vec![api, old]
    }

    fn table(width: usize, styled: bool) -> String {
        match threads_view(&sample(), &opts(width, styled)) {
            Some(View::Table(markdown)) => markdown,
            other => panic!("not a table: {other:?}"),
        }
    }

    #[test]
    fn no_rows_make_no_view() {
        assert_eq!(threads_view(&[], &opts(80, false)), None);
    }

    #[test]
    fn the_table_folds_closing_into_state_and_dashes_zeros() {
        assert_eq!(
            table(200, false),
            concat!(
                "| NAME | STATE | FORK | CREATED | ACTIVE | MSGS | WORKERS | UNREAD |\n",
                "| --- | --- | --- | --- | --- | --- | --- | --- |\n",
                "| api-design | open | #12 | 2d ago | 3h ago | 14 | 1 | 2 |\n",
                "| old-idea | parked 7d ago | #5 | 9d ago | `-` | 0 | `-` | `-` |"
            )
        );
    }

    #[test]
    fn colour_marks_the_name_and_the_state() {
        let styled = table(200, true);
        assert!(styled.contains("**api-design**"), "{styled}");
        assert!(
            styled.contains(&format!("{TONE_DONE}open{TONE_END}")),
            "{styled}"
        );
        assert!(styled.contains("*-*"), "{styled}");
    }

    #[test]
    fn narrow_widths_drop_active_then_unread_then_workers() {
        let header = |width| table(width, false).lines().next().unwrap().to_string();
        assert!(header(66).contains("ACTIVE"));
        assert!(!header(65).contains("ACTIVE") && header(65).contains("UNREAD"));
        assert!(!header(58).contains("UNREAD") && header(58).contains("WORKERS"));
        assert!(!header(51).contains("WORKERS") && header(51).contains("MSGS"));
    }

    #[test]
    fn unparsable_times_show_a_dim_dash_and_future_times_clamp() {
        let mut bad = row("bad", ThreadState::Parked("junk".into()));
        bad.created = "".into();
        bad.active = Some("not a time".into());
        let mut gone = row("gone", ThreadState::Merged("?".into()));
        gone.created = "2026-02-01T00:00:00Z".into();
        let Some(View::Table(markdown)) = threads_view(&[bad, gone], &opts(200, false)) else {
            panic!("not a table");
        };
        assert!(!markdown.contains('?'), "{markdown}");
        assert!(
            markdown.contains("| bad | parked - | #1 | `-` | `-` |"),
            "{markdown}"
        );
        assert!(
            markdown.contains("| gone | merged - | #1 | 0s ago |"),
            "{markdown}"
        );
    }

    #[test]
    fn below_the_smallest_table_each_thread_gets_a_block() {
        let Some(View::Text(lines)) = threads_view(&sample(), &opts(43, false)) else {
            panic!("not text");
        };
        let text = lines_text(&lines);
        assert!(text.starts_with("● api-design · open\n"), "{text}");
        assert!(text.contains("  created  2d ago"), "{text}");
        assert!(text.contains("● old-idea · parked 7d ago"), "{text}");
        assert!(text.contains("  active   -"), "{text}");
    }
}
