use crate::chat::ResetLimits;
use crate::worker_costs::window::instant;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CACHE_TTL_SECS: i64 = 3600;
pub const OWN_COST_LEDGER: u8 = 2;
const SEPARATOR: &str = " · ";
const MISSING: &str = "–";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResetReason {
    Tokens,
    Turns,
    Restart,
}

impl ResetReason {
    pub fn label(self) -> &'static str {
        match self {
            ResetReason::Tokens => "tokens",
            ResetReason::Turns => "turns",
            ResetReason::Restart => "restart",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct UsageLine {
    pub kind: String,
    pub at: String,
    pub cost_usd: Option<f64>,
    pub context_tokens: Option<u64>,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
    pub process_turn: Option<u64>,
    pub request_at: Option<String>,
    pub reset: Option<ResetReason>,
    pub run: Option<String>,
    pub ticket: Option<String>,
    pub worker_id: Option<String>,
    pub session_id: Option<String>,
    pub store: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub reasoning_tokens: Option<u64>,
    pub cache_write_5m: Option<u64>,
    pub cache_write_1h: Option<u64>,
    pub long_prompt: Option<bool>,
    pub per_thread_usage: Option<bool>,
    pub ledger_v: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionCost {
    pub total: f64,
    pub delta: Option<f64>,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessUsage {
    pub process_turn: u64,
    pub context_tokens: Option<u64>,
    pub write_heavy: bool,
    pub request_at: Option<DateTime<Utc>>,
    pub reset: Option<ResetReason>,
    pub cost: Option<SessionCost>,
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessEnv {
    pub now: DateTime<Utc>,
    pub last_ping: Option<DateTime<Utc>>,
    pub live: bool,
    pub limits: ResetLimits,
    pub shift: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessCell {
    pub turn: u64,
    pub max_turns: u64,
    pub ctx_percent: Option<u64>,
    pub cache_minutes: Option<u64>,
    pub write_heavy: bool,
    pub reset: Option<ResetReason>,
    pub shift: Option<usize>,
    pub cost: Option<SessionCost>,
}

pub fn run_id(pid: u32, started: DateTime<Utc>) -> String {
    format!("{pid}-{}", started.to_rfc3339())
}

fn main_rows(usage_jsonl: &str, run: &str) -> Vec<UsageLine> {
    usage_jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
        .filter(|usage| matches!(usage.kind.as_str(), "turn" | "ping"))
        .filter(|usage| usage.run.as_deref() == Some(run))
        .collect()
}

pub fn session_rows(usage_jsonl: &str, run: &str) -> Vec<UsageLine> {
    normalize_costs(main_rows(usage_jsonl, run))
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileStamp {
    pub id: Option<(u64, u64)>,
    pub len: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageTail {
    pub file: Option<(u64, u64)>,
    pub offset: u64,
    pub rows: Vec<UsageLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TailRead {
    pub from: u64,
    pub reset: bool,
}

impl UsageTail {
    pub fn starting_at(stamp: &FileStamp) -> Self {
        Self {
            file: stamp.id,
            offset: stamp.len,
            rows: Vec::new(),
        }
    }
}

pub fn tail_read(tail: &UsageTail, stamp: &FileStamp) -> TailRead {
    let reset = tail.file != stamp.id || stamp.len < tail.offset;
    TailRead {
        from: if reset { 0 } else { tail.offset },
        reset,
    }
}

pub fn absorb_usage(
    tail: UsageTail,
    stamp: &FileStamp,
    read: TailRead,
    chunk: &[u8],
    run: &str,
) -> UsageTail {
    let consumed = chunk
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let mut rows = if read.reset { Vec::new() } else { tail.rows };
    rows.extend(main_rows(&String::from_utf8_lossy(&chunk[..consumed]), run));
    UsageTail {
        file: stamp.id,
        offset: read.from + consumed as u64,
        rows: normalize_costs(rows),
    }
}

pub(crate) fn processes(rows: &[UsageLine]) -> Vec<&[UsageLine]> {
    let starts: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(at, turn)| *at == 0 || turn.process_turn == Some(1))
        .map(|(at, _)| at)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, start)| &rows[*start..starts.get(n + 1).copied().unwrap_or(rows.len())])
        .collect()
}

pub fn turn_cost(total: f64, before: Option<f64>) -> Option<f64> {
    Some(total - before?)
}

fn is_main(row: &UsageLine) -> bool {
    matches!(row.kind.as_str(), "turn" | "ping")
}

pub fn normalize_costs(rows: Vec<UsageLine>) -> Vec<UsageLine> {
    let mut running: BTreeMap<Option<String>, f64> = BTreeMap::new();
    rows.into_iter()
        .map(|row| {
            if !is_main(&row) {
                return row;
            }
            let total = running.entry(row.run.clone()).or_insert(0.0);
            if row.process_turn == Some(1) {
                *total = 0.0;
            }
            let own_cost = row.ledger_v.is_some_and(|v| v >= OWN_COST_LEDGER);
            let own = row.cost_usd.map(|cost| {
                if own_cost {
                    *total += cost;
                    cost
                } else {
                    let own = cost - *total;
                    *total = cost;
                    own
                }
            });
            UsageLine {
                cost_usd: own,
                ledger_v: Some(OWN_COST_LEDGER),
                ..row
            }
        })
        .collect()
}

fn last_delta(rows: &[UsageLine]) -> Option<f64> {
    rows.iter()
        .rfind(|row| row.kind == "turn")?
        .cost_usd
        .filter(|cost| *cost >= 0.0)
}

fn carries_tokens(row: &UsageLine) -> bool {
    row.input_tokens > 0
        || row.cache_creation_input_tokens > 0
        || row.cache_read_input_tokens > 0
        || row.output_tokens > 0
}

fn process_missing(process: &[UsageLine]) -> bool {
    process.iter().all(|row| row.cost_usd.is_none())
        || process
            .iter()
            .rfind(|row| carries_tokens(row))
            .is_some_and(|row| row.cost_usd.is_none())
}

pub fn session_cost(rows: &[UsageLine]) -> Option<SessionCost> {
    let costs: Vec<f64> = rows
        .iter()
        .filter(|row| is_main(row))
        .filter_map(|row| row.cost_usd)
        .collect();
    (!costs.is_empty()).then(|| SessionCost {
        total: costs.iter().sum(),
        delta: last_delta(rows),
        missing: processes(rows)
            .iter()
            .any(|process| process_missing(process)),
    })
}

pub fn restart_write(input: u64, write: u64, read: u64) -> bool {
    let total = input.saturating_add(write).saturating_add(read);
    total > 0 && write.saturating_mul(2) >= total
}

pub fn cache_left_minutes(
    request_at: Option<DateTime<Utc>>,
    last_ping: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    live: bool,
) -> Option<u64> {
    let touched = request_at.max(last_ping).filter(|_| live)?;
    let elapsed = (now - touched).num_seconds();
    let left = u64::try_from(CACHE_TTL_SECS - elapsed.max(0)).ok()?;
    (left > 0).then(|| left.div_ceil(60))
}

pub fn process_usage(rows: &[UsageLine]) -> Option<ProcessUsage> {
    let last = rows.iter().rfind(|row| row.kind == "turn")?;
    Some(ProcessUsage {
        process_turn: last.process_turn.unwrap_or(0),
        context_tokens: last.context_tokens,
        write_heavy: restart_write(
            last.input_tokens,
            last.cache_creation_input_tokens,
            last.cache_read_input_tokens,
        ),
        request_at: last.request_at.as_deref().and_then(instant),
        reset: last.reset,
        cost: session_cost(rows),
    })
}

pub fn process_cell(usage: Option<&ProcessUsage>, env: &ProcessEnv) -> ProcessCell {
    let live = usage.filter(|_| env.live);
    ProcessCell {
        turn: live.map_or(0, |usage| usage.process_turn),
        max_turns: env.limits.turns,
        ctx_percent: live
            .and_then(|usage| usage.context_tokens)
            .and_then(|tokens| (tokens * 100).checked_div(env.limits.tokens)),
        cache_minutes: usage.and_then(|usage| {
            cache_left_minutes(usage.request_at, env.last_ping, env.now, env.live)
        }),
        write_heavy: usage.is_some_and(|usage| usage.write_heavy),
        reset: live.and_then(|usage| usage.reset),
        shift: env.shift,
        cost: usage.and_then(|usage| usage.cost),
    }
}

fn percent_text(cell: &ProcessCell) -> String {
    cell.ctx_percent
        .map_or(MISSING.to_string(), |percent| format!("{percent}%"))
}

fn cache_text(cell: &ProcessCell) -> String {
    match cell.cache_minutes {
        Some(minutes) if cell.write_heavy => format!("{minutes}m!"),
        Some(minutes) => format!("{minutes}m"),
        None => "cold".to_string(),
    }
}

pub fn process_variants(cell: &ProcessCell) -> Vec<String> {
    let turns = format!("{}/{}", cell.turn, cell.max_turns);
    let percent = percent_text(cell);
    let cache = cache_text(cell);
    let prefix = cell.reset.map_or(String::new(), |reason| {
        format!("↻ {}{SEPARATOR}", reason.label())
    });
    let ctx = cell
        .ctx_percent
        .map_or(format!("ctx {MISSING}"), |_| format!("ctx {percent}"));
    let shift = cell.shift.map_or(String::new(), |shift| {
        format!("{SEPARATOR}shift ~{shift} msg")
    });
    vec![
        format!("{prefix}T{turns}{SEPARATOR}{ctx}{SEPARATOR}cache {cache}{shift}"),
        format!("{prefix}T{turns}{SEPARATOR}{percent}{SEPARATOR}{cache}"),
        format!("{turns} {percent} {cache}"),
        cache,
    ]
}

pub fn cost_variants(cost: &SessionCost) -> Vec<String> {
    let total = format!("${:.2}", cost.total);
    match cost.delta {
        Some(delta) => vec![format!("{total} (+{delta:.2})"), total],
        None => vec![total],
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod cost_tests;
#[cfg(test)]
mod normalize_tests;
#[cfg(test)]
mod old_rows_tests;
