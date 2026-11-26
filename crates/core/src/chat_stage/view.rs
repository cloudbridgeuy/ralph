use super::{Item, Kind, STAGE_EXPIRY_LOG_DISTANCE, THREAD_UNRESOLVED};
use crate::chat_help::wrap;
use crate::chat_spec::Keyword;
use crate::chat_status::{ScheduleState, Status, WatchState, WorkerState};
use crate::chat_thread::ThreadName;
use crate::worker::Attention;
use crate::workers_view::{activity_tone, block, dim, plain, span, spans_text, Span, Tone};
use std::fmt;
use unicode_width::UnicodeWidthStr;

const LOG_ROWS_PER_TURN: usize = 2;
const EXPIRY_WARN_TURNS: usize = 5;
const TITLE: &str = "Stage";
const SEPARATOR: &str = " · ";
const INDENT: &str = "  ";
const COLUMN_GAP: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageItemId(String);

impl fmt::Display for StageItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRef(String);

impl fmt::Display for TicketRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StageInput<'a> {
    pub items: &'a [Item],
    pub status: &'a Status,
    pub thread: Option<&'a ThreadName>,
    pub log_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub id: StageItemId,
    pub reference: Option<TicketRef>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Need {
    Decision(ItemRow),
    Worker(WorkerState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenKind {
    Owed,
    Followup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRow {
    pub kind: OpenKind,
    pub row: ItemRow,
    pub turns_left: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    Workers(Vec<WorkerState>),
    Watches(Vec<WatchState>),
    Schedule(Vec<ScheduleState>),
    Inbox(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageView {
    pub thread: Option<ThreadName>,
    pub needs: Vec<Need>,
    pub open: Vec<OpenRow>,
    pub running: Vec<Fact>,
}

pub fn turns_left(item: &Item, log_len: usize) -> usize {
    STAGE_EXPIRY_LOG_DISTANCE
        .saturating_sub(log_len.saturating_sub(item.turn))
        .div_ceil(LOG_ROWS_PER_TURN)
}

fn item_row(item: &Item) -> ItemRow {
    ItemRow {
        id: StageItemId(item.id.clone()),
        reference: item.reference.clone().map(TicketRef),
        text: item.text.clone(),
    }
}

fn open_row(item: &Item, log_len: usize) -> Option<OpenRow> {
    let kind = match item.kind {
        Kind::Decision => return None,
        Kind::Owed => OpenKind::Owed,
        Kind::Followup => OpenKind::Followup,
    };
    Some(OpenRow {
        kind,
        row: item_row(item),
        turns_left: turns_left(item, log_len),
    })
}

pub fn stage_view(input: &StageInput) -> Option<StageView> {
    let status = input.status;
    let (needing, other): (Vec<WorkerState>, Vec<WorkerState>) = status
        .workers
        .iter()
        .cloned()
        .partition(|worker| worker.state.attention() == Attention::NeedsUser);
    let needs: Vec<Need> = input
        .items
        .iter()
        .filter(|item| item.kind == Kind::Decision)
        .map(|item| Need::Decision(item_row(item)))
        .chain(needing.into_iter().map(Need::Worker))
        .collect();
    let open: Vec<OpenRow> = input
        .items
        .iter()
        .filter_map(|item| open_row(item, input.log_len))
        .collect();
    let running: Vec<Fact> = [
        (!other.is_empty()).then_some(Fact::Workers(other)),
        (!status.watches.is_empty()).then(|| Fact::Watches(status.watches.clone())),
        (!status.schedule.is_empty()).then(|| Fact::Schedule(status.schedule.clone())),
        (status.unread > 0).then_some(Fact::Inbox(status.unread)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let shown =
        input.thread.is_some() || !needs.is_empty() || !open.is_empty() || !running.is_empty();
    shown.then(|| StageView {
        thread: input.thread.cloned(),
        needs,
        open,
        running,
    })
}

impl OpenKind {
    fn kind(self) -> Kind {
        match self {
            OpenKind::Owed => Kind::Owed,
            OpenKind::Followup => Kind::Followup,
        }
    }
}

struct ItemLine<'a> {
    glyph: Span,
    kind: Span,
    row: &'a ItemRow,
    turns_left: Option<usize>,
}

struct Columns {
    id: usize,
    kind: usize,
    width: usize,
}

fn padded(text: &str, column: usize) -> String {
    format!("{text:<column$}")
}

fn title_lines(thread: Option<&ThreadName>, width: usize) -> Vec<Vec<Span>> {
    let Some(name) = thread else {
        return vec![vec![span(Tone::Name, TITLE)]];
    };
    let title = vec![
        span(Tone::Name, TITLE),
        plain(format!("{SEPARATOR}thread ")),
        span(Tone::Name, name.as_str()),
    ];
    let tail = span(Tone::Attention, THREAD_UNRESOLVED);
    let joined = spans_text(&title).width() + SEPARATOR.width() + THREAD_UNRESOLVED.width();
    if joined <= width {
        vec![title.into_iter().chain([plain(SEPARATOR), tail]).collect()]
    } else {
        vec![title, vec![plain(INDENT), tail]]
    }
}

fn item_lines(line: &ItemLine, columns: &Columns) -> Vec<Vec<Span>> {
    let head = vec![
        plain(INDENT),
        line.glyph.clone(),
        plain(" "),
        span(Tone::Name, padded(&line.row.id.to_string(), columns.id)),
        span(line.kind.tone, padded(&line.kind.text, columns.kind)),
    ];
    let indent = spans_text(&head).width();
    let room = columns.width.saturating_sub(indent);
    let reference = line.row.reference.as_ref().map(TicketRef::to_string);
    let text = match &reference {
        Some(reference) => format!("{reference}: {}", line.row.text),
        None => line.row.text.clone(),
    };
    let wrapped = wrap(&text, room);
    let mut lines: Vec<Vec<Span>> = wrapped
        .iter()
        .enumerate()
        .map(|(row, part)| {
            let start = if row == 0 {
                head.clone()
            } else {
                vec![plain(" ".repeat(indent))]
            };
            let named = reference
                .as_deref()
                .filter(|_| row == 0)
                .and_then(|reference| Some((reference, part.strip_prefix(reference)?)));
            let text = match named {
                Some((reference, rest)) => vec![span(Tone::Active, reference), plain(rest)],
                None => vec![plain(part.as_str())],
            };
            start.into_iter().chain(text).collect()
        })
        .collect();
    if let Some(turns) = line.turns_left {
        let tone = if turns <= EXPIRY_WARN_TURNS {
            Tone::Attention
        } else {
            Tone::Dim
        };
        let left = format!("~{turns} turns left");
        let fits = wrapped
            .last()
            .is_some_and(|last| last.width() + SEPARATOR.width() + left.width() <= room);
        match lines.last_mut() {
            Some(last) if fits => last.extend([dim(SEPARATOR), span(tone, left)]),
            _ => lines.push(vec![plain(" ".repeat(indent)), span(tone, left)]),
        }
    }
    lines
}

fn need_lines(needs: &[Need], columns: &Columns) -> Vec<Vec<Span>> {
    let worker_column = needs
        .iter()
        .filter_map(|need| match need {
            Need::Worker(worker) => Some(worker.id.width() + COLUMN_GAP),
            Need::Decision(_) => None,
        })
        .max()
        .unwrap_or(0);
    needs
        .iter()
        .flat_map(|need| match need {
            Need::Decision(row) => item_lines(
                &ItemLine {
                    glyph: span(Tone::Attention, "◆"),
                    kind: span(Tone::Attention, Kind::Decision.word()),
                    row,
                    turns_left: None,
                },
                columns,
            ),
            Need::Worker(worker) => {
                let tone = activity_tone(worker.state);
                vec![vec![
                    plain(INDENT),
                    span(tone, "▲"),
                    plain(" "),
                    span(Tone::Name, padded(&worker.id, worker_column)),
                    span(tone, worker.state.as_str()),
                ]]
            }
        })
        .collect()
}

fn open_lines(open: &[OpenRow], columns: &Columns) -> Vec<Vec<Span>> {
    open.iter()
        .flat_map(|open| {
            let (glyph, tone) = match open.kind {
                OpenKind::Owed => (span(Tone::Active, "●"), Tone::Plain),
                OpenKind::Followup => (span(Tone::Dim, "○"), Tone::Dim),
            };
            item_lines(
                &ItemLine {
                    glyph,
                    kind: span(tone, open.kind.kind().word()),
                    row: &open.row,
                    turns_left: Some(open.turns_left),
                },
                columns,
            )
        })
        .collect()
}

fn separated(entries: impl Iterator<Item = Vec<Span>>) -> Vec<Span> {
    entries
        .enumerate()
        .flat_map(|(position, entry)| {
            (position > 0)
                .then(|| plain(SEPARATOR))
                .into_iter()
                .chain(entry)
        })
        .collect()
}

fn fact_line(fact: &Fact) -> (&'static str, Vec<Span>) {
    match fact {
        Fact::Workers(workers) => (
            "workers",
            separated(workers.iter().map(|worker| {
                vec![
                    plain(format!("{} ", worker.id)),
                    span(activity_tone(worker.state), worker.state.as_str()),
                ]
            })),
        ),
        Fact::Watches(watches) => (
            "watches",
            separated(watches.iter().map(|watch| {
                let left = watch.left.as_ref().map(|left| dim(format!(" {left}")));
                std::iter::once(plain(watch.id.as_str()))
                    .chain(left)
                    .collect()
            })),
        ),
        Fact::Schedule(schedule) => (
            "schedule",
            separated(
                schedule
                    .iter()
                    .map(|task| vec![plain(task.id.as_str()), dim(format!(" {}", task.time))]),
            ),
        ),
        Fact::Inbox(unread) => ("inbox", vec![plain(format!("{unread} unread"))]),
    }
}

fn section(title: &str, lines: Vec<Vec<Span>>) -> Vec<Vec<Span>> {
    if lines.is_empty() {
        return lines;
    }
    [Vec::new(), vec![span(Tone::Name, title)]]
        .into_iter()
        .chain(lines)
        .collect()
}

pub fn view_lines(view: &StageView, width: usize) -> Vec<Vec<Span>> {
    let rows = view
        .needs
        .iter()
        .filter_map(|need| match need {
            Need::Decision(row) => Some(row),
            Need::Worker(_) => None,
        })
        .chain(view.open.iter().map(|open| &open.row));
    let columns = Columns {
        id: rows
            .map(|row| row.id.to_string().width() + COLUMN_GAP)
            .max()
            .unwrap_or(0),
        kind: Kind::ALL
            .iter()
            .map(|kind| kind.word().width() + COLUMN_GAP)
            .max()
            .unwrap_or(0),
        width,
    };
    let facts: Vec<(&str, Vec<Span>)> = view.running.iter().map(fact_line).collect();
    let running = if facts.is_empty() {
        Vec::new()
    } else {
        std::iter::once(Vec::new())
            .chain(block(vec![span(Tone::Name, "Running")], &facts, width))
            .collect()
    };
    title_lines(view.thread.as_ref(), width)
        .into_iter()
        .chain(section("Needs you", need_lines(&view.needs, &columns)))
        .chain(section("Open", open_lines(&view.open, &columns)))
        .chain(running)
        .collect()
}

#[cfg(test)]
mod tests;
