use chrono::DateTime;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Read,
    Zoom,
    Recall,
    Search,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Mcp,
    Slash,
    Cli,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Empty,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub index: usize,
    pub at: i64,
    pub run: Option<String>,
    pub turn: Option<String>,
    pub source: Source,
    pub tool: Tool,
    pub scope: Option<String>,
    pub args: Value,
    pub outcome: Outcome,
    pub hits: Option<u64>,
    pub node_size: Option<u64>,
    pub depth: Option<u64>,
    pub oldest_hit: Option<u64>,
    pub hit_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Ids(Vec<String>),
    First(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnRow {
    pub index: usize,
    pub at: i64,
    pub run: Option<String>,
    pub turn: Option<String>,
    pub scope: Option<String>,
    pub history_calls: Option<u64>,
    pub view: View,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimingTurn {
    pub index: usize,
    pub at: i64,
    pub run: Option<String>,
    pub turn_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub calls: Vec<Call>,
    pub turns: Vec<TurnRow>,
    pub skipped: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimingParsed {
    pub turns: Vec<TimingTurn>,
    pub skipped: usize,
}

#[derive(Deserialize)]
struct RawRow {
    at: Option<String>,
    run: Option<String>,
    turn: Option<String>,
    event: Option<String>,
    source: Option<String>,
    tool: Option<String>,
    scope: Option<String>,
    args: Option<Value>,
    outcome: Option<String>,
    hits: Option<u64>,
    node_size: Option<u64>,
    depth: Option<u64>,
    oldest_hit: Option<u64>,
    hit_ids: Option<Vec<String>>,
    history_calls: Option<u64>,
    view_ids: Option<Vec<String>>,
    view_first: Option<String>,
    turn_ms: Option<u64>,
}

fn read_row(line: &str) -> Option<(RawRow, i64)> {
    let row: RawRow = serde_json::from_str(line).ok()?;
    let at = DateTime::parse_from_rfc3339(row.at.as_deref()?)
        .ok()?
        .timestamp();
    Some((row, at))
}

fn tool_of(name: Option<&str>) -> Tool {
    match name {
        Some("read") => Tool::Read,
        Some("zoom") => Tool::Zoom,
        Some("recall") => Tool::Recall,
        Some("search") => Tool::Search,
        _ => Tool::Unknown,
    }
}

fn source_of(name: Option<&str>) -> Source {
    match name {
        Some("mcp") => Source::Mcp,
        Some("slash") => Source::Slash,
        Some("cli") => Source::Cli,
        _ => Source::Unknown,
    }
}

fn outcome_of(name: Option<&str>) -> Outcome {
    match name {
        Some("ok") => Outcome::Ok,
        Some("empty") => Outcome::Empty,
        Some("error") => Outcome::Error,
        _ => Outcome::Unknown,
    }
}

fn view_of(row: &mut RawRow) -> View {
    match (row.view_ids.take(), row.view_first.take()) {
        (Some(ids), _) => View::Ids(ids),
        (None, Some(first)) => View::First(first),
        (None, None) => View::Ids(Vec::new()),
    }
}

fn call_of(mut row: RawRow, index: usize, at: i64) -> Call {
    Call {
        index,
        at,
        run: row.run.take(),
        turn: row.turn.take(),
        source: source_of(row.source.as_deref()),
        tool: tool_of(row.tool.as_deref()),
        scope: row.scope.take(),
        args: row.args.take().unwrap_or(Value::Null),
        outcome: outcome_of(row.outcome.as_deref()),
        hits: row.hits,
        node_size: row.node_size,
        depth: row.depth,
        oldest_hit: row.oldest_hit,
        hit_ids: row.hit_ids.take(),
    }
}

fn turn_of(mut row: RawRow, index: usize, at: i64) -> TurnRow {
    let view = view_of(&mut row);
    TurnRow {
        index,
        at,
        run: row.run.take(),
        turn: row.turn.take(),
        scope: row.scope.take(),
        history_calls: row.history_calls,
        view,
    }
}

pub fn parse_history(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((row, at)) = read_row(line) else {
            parsed.skipped += 1;
            continue;
        };
        match row.event.as_deref() {
            Some("call") => parsed.calls.push(call_of(row, index, at)),
            Some("turn") => parsed.turns.push(turn_of(row, index, at)),
            _ => parsed.skipped += 1,
        }
    }
    parsed
}

pub fn parse_timing(text: &str) -> TimingParsed {
    let mut parsed = TimingParsed::default();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((mut row, at)) = read_row(line) else {
            parsed.skipped += 1;
            continue;
        };
        if row.event.as_deref() == Some("turn") {
            parsed.turns.push(TimingTurn {
                index,
                at,
                run: row.run.take(),
                turn_ms: row.turn_ms,
            });
        }
    }
    parsed
}
