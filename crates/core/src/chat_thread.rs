use crate::chat::ResetLimits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const CACHE_TTL_SECS: i64 = 3600;
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
    pub worker_id: Option<String>,
    pub session_id: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub reasoning_tokens: Option<u64>,
    pub cache_write_5m: Option<u64>,
    pub cache_write_1h: Option<u64>,
    pub long_prompt: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionCost {
    pub total: f64,
    pub delta: Option<f64>,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadUsage {
    pub process_turn: u64,
    pub context_tokens: Option<u64>,
    pub write_heavy: bool,
    pub request_at: Option<DateTime<Utc>>,
    pub reset: Option<ResetReason>,
    pub cost: Option<SessionCost>,
}

#[derive(Debug, Clone, Copy)]
pub struct ThreadEnv {
    pub now: DateTime<Utc>,
    pub last_ping: Option<DateTime<Utc>>,
    pub live: bool,
    pub limits: ResetLimits,
    pub shift: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadCell {
    pub turn: u64,
    pub max_turns: u64,
    pub ctx_percent: Option<u64>,
    pub cache_minutes: Option<u64>,
    pub write_heavy: bool,
    pub reset: Option<ResetReason>,
    pub shift: Option<usize>,
    pub cost: Option<SessionCost>,
}

fn parse_time(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

pub fn run_id(pid: u32, started: DateTime<Utc>) -> String {
    format!("{pid}-{}", started.to_rfc3339())
}

pub fn session_rows(usage_jsonl: &str, run: &str) -> Vec<UsageLine> {
    usage_jsonl
        .lines()
        .filter_map(|line| serde_json::from_str::<UsageLine>(line).ok())
        .filter(|usage| matches!(usage.kind.as_str(), "turn" | "ping"))
        .filter(|usage| usage.run.as_deref() == Some(run))
        .collect()
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
    rows.extend(session_rows(
        &String::from_utf8_lossy(&chunk[..consumed]),
        run,
    ));
    UsageTail {
        file: stamp.id,
        offset: read.from + consumed as u64,
        rows,
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

fn last_delta(rows: &[UsageLine]) -> Option<f64> {
    let at = rows.iter().rposition(|row| row.kind == "turn")?;
    let own = rows[at].cost_usd?;
    let base = if rows[at].process_turn == Some(1) {
        0.0
    } else {
        rows[..at]
            .iter()
            .rev()
            .scan(false, |done, row| {
                let previous = (!*done).then_some(row);
                *done = row.process_turn == Some(1);
                previous
            })
            .find_map(|row| row.cost_usd)?
    };
    Some(own - base).filter(|delta| *delta >= 0.0)
}

pub fn session_cost(rows: &[UsageLine]) -> Option<SessionCost> {
    let processes = processes(rows);
    let totals: Vec<f64> = processes
        .iter()
        .filter_map(|process| process.iter().rev().find_map(|turn| turn.cost_usd))
        .collect();
    (!totals.is_empty()).then(|| SessionCost {
        total: totals.iter().sum(),
        delta: last_delta(rows),
        missing: totals.len() < processes.len(),
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

pub fn thread_usage(rows: &[UsageLine]) -> Option<ThreadUsage> {
    let last = rows.iter().rfind(|row| row.kind == "turn")?;
    Some(ThreadUsage {
        process_turn: last.process_turn.unwrap_or(0),
        context_tokens: last.context_tokens,
        write_heavy: restart_write(
            last.input_tokens,
            last.cache_creation_input_tokens,
            last.cache_read_input_tokens,
        ),
        request_at: last.request_at.as_deref().and_then(parse_time),
        reset: last.reset,
        cost: session_cost(rows),
    })
}

pub fn thread_cell(usage: Option<&ThreadUsage>, env: &ThreadEnv) -> ThreadCell {
    let live = usage.filter(|_| env.live);
    ThreadCell {
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

fn percent_text(cell: &ThreadCell) -> String {
    cell.ctx_percent
        .map_or(MISSING.to_string(), |percent| format!("{percent}%"))
}

fn cache_text(cell: &ThreadCell) -> String {
    match cell.cache_minutes {
        Some(minutes) if cell.write_heavy => format!("{minutes}m!"),
        Some(minutes) => format!("{minutes}m"),
        None => "cold".to_string(),
    }
}

pub fn thread_variants(cell: &ThreadCell) -> Vec<String> {
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
mod tests {
    use super::*;

    const LIMITS: ResetLimits = ResetLimits {
        turns: 30,
        tokens: 140_000,
    };

    fn time(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    const RUN: &str = "7-run";

    fn run_line(run: &str, secs: i64, process_turn: u64, cost: f64) -> String {
        format!(
            r#"{{"at":"{}","kind":"turn","cost_usd":{cost},"process_turn":{process_turn},"run":"{run}"}}"#,
            time(secs).to_rfc3339()
        )
    }

    fn turn_line(secs: i64, process_turn: u64, cost: f64) -> String {
        run_line(RUN, secs, process_turn, cost)
    }

    fn ping_line(secs: i64, cost: f64) -> String {
        format!(
            r#"{{"at":"{}","kind":"ping","cost_usd":{cost},"run":"{RUN}"}}"#,
            time(secs).to_rfc3339()
        )
    }

    fn ping_line_without_cost(secs: i64) -> String {
        format!(
            r#"{{"at":"{}","kind":"ping","run":"{RUN}"}}"#,
            time(secs).to_rfc3339()
        )
    }

    fn lines(rows: &[String]) -> String {
        rows.iter().map(|row| format!("{row}\n")).collect()
    }

    fn usage(request_at: Option<DateTime<Utc>>) -> ThreadUsage {
        ThreadUsage {
            process_turn: 12,
            context_tokens: Some(86_800),
            write_heavy: false,
            request_at,
            reset: None,
            cost: None,
        }
    }

    fn env(live: bool, shift: Option<usize>) -> ThreadEnv {
        ThreadEnv {
            now: time(0),
            last_ping: None,
            live,
            limits: LIMITS,
            shift,
        }
    }

    fn cell() -> ThreadCell {
        ThreadCell {
            turn: 12,
            max_turns: 30,
            ctx_percent: Some(62),
            cache_minutes: Some(41),
            write_heavy: true,
            reset: None,
            shift: Some(9),
            cost: None,
        }
    }

    #[test]
    fn session_cost_sums_the_last_cost_of_each_process() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            turn_line(2, 2, 3.0),
            turn_line(3, 1, 0.5),
            turn_line(4, 2, 1.25),
        ]);
        let cost = session_cost(&session_rows(&text, RUN)).unwrap();
        assert_eq!(cost.total, 4.25);
        assert_eq!(cost.delta, Some(0.75));
    }

    #[test]
    fn session_cost_does_not_sum_running_totals() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            turn_line(2, 2, 3.0),
            turn_line(3, 3, 6.0),
        ]);
        assert_eq!(session_cost(&session_rows(&text, RUN)).unwrap().total, 6.0);
    }

    #[test]
    fn session_rows_keep_only_the_own_run_when_two_runs_interleave() {
        let text = lines(&[
            run_line("other", 1, 1, 9.0),
            turn_line(2, 1, 2.0),
            run_line("other", 3, 2, 11.0),
            turn_line(4, 2, 3.0),
        ]);
        let turns = session_rows(&text, RUN);
        assert_eq!(turns.len(), 2);
        assert_eq!(session_cost(&turns).unwrap().total, 3.0);
        assert_eq!(session_rows(&text, "other").len(), 2);
        assert_eq!(session_rows(&text, "missing").len(), 0);
    }

    #[test]
    fn session_rows_parse_lines_that_carry_a_store() {
        let line = format!(
            r#"{{"at":"{}","kind":"turn","process_turn":1,"run":"{RUN}","store":"/s"}}"#,
            time(1).to_rfc3339()
        );
        assert_eq!(session_rows(&line, RUN).len(), 1);
    }

    #[test]
    fn session_rows_exclude_lines_without_a_run() {
        let bare = format!(
            r#"{{"at":"{}","kind":"turn","cost_usd":9.0,"process_turn":1}}"#,
            time(1).to_rfc3339()
        );
        let text = lines(&[bare, turn_line(2, 1, 2.0)]);
        let turns = session_rows(&text, RUN);
        assert_eq!(turns.len(), 1);
        assert_eq!(session_cost(&turns).unwrap().total, 2.0);
    }

    #[test]
    fn session_rows_skip_worker_usage_lines() {
        let worker = format!(
            r#"{{"at":"{}","kind":"worker","worker_id":"w1","input_tokens":5,"run":"{RUN}"}}"#,
            time(1).to_rfc3339()
        );
        let text = lines(&[worker, turn_line(2, 1, 2.0)]);
        assert_eq!(session_rows(&text, RUN).len(), 1);
    }

    #[test]
    fn run_id_joins_the_pid_and_the_start_time() {
        assert_eq!(run_id(42, time(0)), format!("42-{}", time(0).to_rfc3339()));
    }

    #[test]
    fn session_cost_excludes_nap_lines_and_garbage() {
        let nap = format!(
            r#"{{"at":"{}","kind":"nap","cost_usd":7.0,"run":"7-run"}}"#,
            time(2).to_rfc3339()
        );
        let text = lines(&[turn_line(1, 1, 2.0), nap, "not json".to_string()]);
        let turns = session_rows(&text, RUN);
        assert_eq!(session_cost(&turns).unwrap().total, 2.0);
    }

    #[test]
    fn session_cost_marks_a_process_without_cost() {
        let bare = format!(
            r#"{{"at":"{}","kind":"turn","process_turn":1,"run":"{RUN}"}}"#,
            time(2).to_rfc3339()
        );
        let partial = lines(&[turn_line(1, 1, 1.0), bare]);
        let cost = session_cost(&session_rows(&partial, RUN)).unwrap();
        assert_eq!(cost.total, 1.0);
        assert!(cost.missing);
        let full = lines(&[turn_line(1, 1, 1.0), turn_line(2, 1, 2.0)]);
        assert!(!session_cost(&session_rows(&full, RUN)).unwrap().missing);
    }

    #[test]
    fn session_cost_is_absent_without_cost() {
        let bare = format!(
            r#"{{"at":"{}","kind":"turn","process_turn":1,"run":"7-run"}}"#,
            time(1).to_rfc3339()
        );
        assert_eq!(session_cost(&session_rows(&bare, RUN)), None);
        assert_eq!(session_cost(&[]), None);
    }

    #[test]
    fn delta_is_the_own_cost_on_a_first_process_turn() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            turn_line(2, 2, 3.0),
            turn_line(3, 1, 0.4),
        ]);
        let cost = session_cost(&session_rows(&text, RUN)).unwrap();
        assert_eq!(cost.delta, Some(0.4));
    }

    #[test]
    fn delta_is_absent_when_the_cost_goes_down() {
        let text = lines(&[turn_line(1, 1, 2.0), turn_line(2, 2, 1.0)]);
        let cost = session_cost(&session_rows(&text, RUN)).unwrap();
        assert_eq!(cost.delta, None);
    }

    #[test]
    fn pings_join_their_process_and_the_last_total_wins() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            ping_line(2, 1.2),
            turn_line(3, 2, 2.0),
            turn_line(4, 1, 0.5),
        ]);
        let rows = session_rows(&text, RUN);
        assert_eq!(rows.len(), 4);
        assert_eq!(session_cost(&rows).unwrap().total, 2.5);
    }

    #[test]
    fn a_process_ending_on_a_ping_totals_the_ping_cost() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            turn_line(2, 2, 2.0),
            ping_line(3, 2.4),
        ]);
        assert_eq!(session_cost(&session_rows(&text, RUN)).unwrap().total, 2.4);
    }

    #[test]
    fn the_delta_of_a_turn_after_a_ping_excludes_the_ping() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            ping_line(2, 1.2),
            turn_line(3, 2, 1.5),
        ]);
        let cost = session_cost(&session_rows(&text, RUN)).unwrap();
        assert_eq!(cost.total, 1.5);
        assert!((cost.delta.unwrap() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn the_delta_skips_a_ping_without_cost_back_to_the_turn_before() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            ping_line_without_cost(2),
            turn_line(3, 2, 1.5),
        ]);
        let cost = session_cost(&session_rows(&text, RUN)).unwrap();
        assert_eq!(cost.delta, Some(0.5));
    }

    #[test]
    fn the_delta_stays_on_the_last_turn_when_a_ping_follows() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            turn_line(2, 2, 1.5),
            ping_line(3, 1.7),
        ]);
        let cost = session_cost(&session_rows(&text, RUN)).unwrap();
        assert_eq!(cost.total, 1.7);
        assert_eq!(cost.delta, Some(0.5));
    }

    #[test]
    fn worker_rows_are_left_out_of_turns_rows_and_cost() {
        let worker = format!(
            r#"{{"at":"{}","kind":"worker","worker_id":"w","cost_usd":9.0,"run":"{RUN}"}}"#,
            time(2).to_rfc3339()
        );
        let text = lines(&[
            turn_line(1, 1, 1.0),
            worker,
            ping_line(3, 1.2),
            turn_line(4, 2, 1.5),
        ]);
        let rows = session_rows(&text, RUN);
        assert_eq!(rows.len(), 3);
        assert_eq!(session_cost(&rows).unwrap().total, 1.5);
        assert_eq!(
            thread_usage(&session_rows(&text, RUN))
                .unwrap()
                .cost
                .unwrap()
                .total,
            1.5
        );
    }

    #[test]
    fn thread_usage_reads_the_last_turn_not_the_last_ping() {
        let text = lines(&[
            turn_line(1, 1, 1.0),
            turn_line(2, 2, 1.5),
            ping_line(3, 1.7),
        ]);
        let usage = thread_usage(&session_rows(&text, RUN)).unwrap();
        assert_eq!(usage.process_turn, 2);
        assert_eq!(usage.cost.unwrap().total, 1.7);
    }

    const FILE: Option<(u64, u64)> = Some((1, 10));

    fn stamp(file: Option<(u64, u64)>, text: &str) -> FileStamp {
        FileStamp {
            id: file,
            len: text.len() as u64,
        }
    }

    fn read_all(tail: UsageTail, file: Option<(u64, u64)>, text: &str) -> UsageTail {
        let stamp = stamp(file, text);
        let read = tail_read(&tail, &stamp);
        let chunk = &text.as_bytes()[read.from as usize..];
        absorb_usage(tail, &stamp, read, chunk, RUN)
    }

    fn costs(tail: &UsageTail) -> Vec<Option<f64>> {
        tail.rows.iter().map(|row| row.cost_usd).collect()
    }

    #[test]
    fn two_absorbs_equal_one_full_parse() {
        let first = lines(&[turn_line(1, 1, 1.0), run_line("other", 2, 1, 9.0)]);
        let all = format!(
            "{first}{}",
            lines(&[ping_line(3, 1.2), turn_line(4, 2, 2.0)])
        );
        let tail = read_all(
            UsageTail::starting_at(&FileStamp { id: FILE, len: 0 }),
            FILE,
            &first,
        );
        let tail = read_all(tail, FILE, &all);
        let full = read_all(
            UsageTail::starting_at(&FileStamp { id: FILE, len: 0 }),
            FILE,
            &all,
        );
        assert_eq!(tail, full);
        assert_eq!(tail.offset, all.len() as u64);
        assert_eq!(costs(&tail), vec![Some(1.0), Some(1.2), Some(2.0)]);
    }

    #[test]
    fn a_torn_last_line_waits_for_the_next_read() {
        let whole = lines(&[turn_line(1, 1, 1.0), turn_line(2, 2, 2.0)]);
        let cut = whole.len() - 10;
        let start = UsageTail::starting_at(&FileStamp { id: FILE, len: 0 });
        let tail = read_all(start, FILE, &whole[..cut]);
        assert_eq!(costs(&tail), vec![Some(1.0)]);
        assert_eq!(tail.offset, whole.find('\n').unwrap() as u64 + 1);
        let tail = read_all(tail, FILE, &whole);
        assert_eq!(costs(&tail), vec![Some(1.0), Some(2.0)]);
        assert_eq!(tail.offset, whole.len() as u64);
    }

    #[test]
    fn a_shorter_file_resets_and_rereads() {
        let long = lines(&[turn_line(1, 1, 1.0), turn_line(2, 2, 2.0)]);
        let short = lines(&[turn_line(3, 1, 5.0)]);
        let start = UsageTail::starting_at(&FileStamp { id: FILE, len: 0 });
        let tail = read_all(start, FILE, &long);
        assert!(tail_read(&tail, &stamp(FILE, &short)).reset);
        let tail = read_all(tail, FILE, &short);
        assert_eq!(costs(&tail), vec![Some(5.0)]);
    }

    #[test]
    fn a_new_inode_resets_and_rereads() {
        let old = lines(&[turn_line(1, 1, 1.0)]);
        let new = lines(&[turn_line(2, 1, 4.0), turn_line(3, 2, 6.0)]);
        let start = UsageTail::starting_at(&FileStamp { id: FILE, len: 0 });
        let tail = read_all(start, FILE, &old);
        let other = Some((1, 11));
        assert_eq!(
            tail_read(&tail, &stamp(other, &new)),
            TailRead {
                from: 0,
                reset: true
            }
        );
        let tail = read_all(tail, other, &new);
        assert_eq!(costs(&tail), vec![Some(4.0), Some(6.0)]);
        assert_eq!(tail.file, other);
    }

    #[test]
    fn an_unchanged_file_reads_on_from_the_offset() {
        let text = lines(&[turn_line(1, 1, 1.0)]);
        let tail = read_all(
            UsageTail::starting_at(&FileStamp { id: FILE, len: 0 }),
            FILE,
            &text,
        );
        assert_eq!(
            tail_read(&tail, &stamp(FILE, &text)),
            TailRead {
                from: text.len() as u64,
                reset: false
            }
        );
    }

    #[test]
    fn interleaved_runs_keep_only_the_own_rows_across_absorbs() {
        let first = lines(&[run_line("other", 1, 1, 9.0), turn_line(2, 1, 2.0)]);
        let all = format!(
            "{first}{}",
            lines(&[run_line("other", 3, 2, 11.0), turn_line(4, 2, 3.0)])
        );
        let tail = read_all(
            UsageTail::starting_at(&FileStamp { id: FILE, len: 0 }),
            FILE,
            &first,
        );
        let tail = read_all(tail, FILE, &all);
        assert_eq!(costs(&tail), vec![Some(2.0), Some(3.0)]);
    }

    #[test]
    fn a_ping_after_a_turn_counts_in_the_session_cost() {
        let first = lines(&[turn_line(1, 1, 1.0)]);
        let all = format!("{first}{}", lines(&[ping_line(2, 1.2)]));
        let tail = read_all(
            UsageTail::starting_at(&FileStamp { id: FILE, len: 0 }),
            FILE,
            &first,
        );
        let tail = read_all(tail, FILE, &all);
        let usage = thread_usage(&tail.rows).unwrap();
        assert_eq!(usage.cost.unwrap().total, 1.2);
    }

    #[test]
    fn a_start_offset_inside_a_line_skips_the_fragment() {
        let text = lines(&[turn_line(1, 1, 1.0), turn_line(2, 2, 2.0)]);
        let mid = FileStamp { id: FILE, len: 20 };
        let tail = read_all(UsageTail::starting_at(&mid), FILE, &text);
        assert_eq!(costs(&tail), vec![Some(2.0)]);
        assert_eq!(tail.offset, text.len() as u64);
    }

    #[test]
    fn starting_at_a_stamp_begins_at_its_end_with_no_rows() {
        let tail = UsageTail::starting_at(&FileStamp { id: FILE, len: 7 });
        assert_eq!((tail.file, tail.offset, tail.rows.len()), (FILE, 7, 0));
    }

    #[test]
    fn a_chunk_without_a_newline_consumes_nothing() {
        let stamp = FileStamp { id: FILE, len: 5 };
        let tail = UsageTail::starting_at(&FileStamp { id: FILE, len: 0 });
        let read = tail_read(&tail, &stamp);
        let tail = absorb_usage(tail, &stamp, read, b"{\"kind", RUN);
        assert_eq!(tail.offset, 0);
        assert!(tail.rows.is_empty());
    }

    #[test]
    fn cache_minutes_follow_the_one_hour_window() {
        let at = Some(time(0));
        let left = |elapsed: i64| cache_left_minutes(at, None, time(elapsed), true);
        assert_eq!(left(0), Some(60));
        assert_eq!(left(1), Some(60));
        assert_eq!(left(60), Some(59));
        assert_eq!(left(3599), Some(1));
        assert_eq!(left(3600), None);
        assert_eq!(left(4000), None);
        assert_eq!(left(-30), Some(60));
    }

    #[test]
    fn cache_is_cold_without_a_request_or_a_live_process() {
        assert_eq!(cache_left_minutes(None, None, time(1), true), None);
        assert_eq!(
            cache_left_minutes(Some(time(0)), None, time(1), false),
            None
        );
        assert_eq!(
            cache_left_minutes(None, Some(time(0)), time(3600), true),
            None
        );
    }

    #[test]
    fn cache_counts_from_the_later_of_the_reply_and_the_ping() {
        let left = |request: Option<i64>, ping: Option<i64>, live| {
            cache_left_minutes(request.map(time), ping.map(time), time(3000), live)
        };
        assert_eq!(left(Some(0), Some(2400), true), Some(50));
        assert_eq!(left(Some(2400), Some(0), true), Some(50));
        assert_eq!(left(None, Some(2400), true), Some(50));
        assert_eq!(left(None, None, true), None);
        assert_eq!(left(Some(0), Some(2400), false), None);
    }

    #[test]
    fn restart_write_ignores_a_tool_heavy_turn() {
        assert!(!restart_write(4, 7_610, 27_154));
    }

    #[test]
    fn restart_write_flags_a_cold_restart() {
        assert!(restart_write(4, 18_000, 0));
    }

    #[test]
    fn restart_write_flags_exactly_half() {
        assert!(restart_write(0, 50, 50));
        assert!(!restart_write(0, 49, 51));
    }

    #[test]
    fn restart_write_is_off_without_tokens() {
        assert!(!restart_write(0, 0, 0));
    }

    #[test]
    fn thread_usage_reads_the_last_turn_of_this_session() {
        let last = format!(
            r#"{{"at":"{}","kind":"turn","cost_usd":2.0,"context_tokens":1000,"input_tokens":4,"cache_creation_input_tokens":600,"cache_read_input_tokens":100,"process_turn":1,"request_at":"{}","reset":"turns","run":"7-run"}}"#,
            time(5).to_rfc3339(),
            time(4).to_rfc3339()
        );
        let text = lines(&[turn_line(1, 4, 1.0), last]);
        let data = thread_usage(&session_rows(&text, RUN)).unwrap();
        assert_eq!(data.process_turn, 1);
        assert_eq!(data.context_tokens, Some(1000));
        assert!(data.write_heavy);
        assert_eq!(data.request_at, Some(time(4)));
        assert_eq!(data.reset, Some(ResetReason::Turns));
        assert_eq!(data.cost.unwrap().total, 3.0);
        assert_eq!(thread_usage(&session_rows(&text, "other")), None);
        assert_eq!(thread_usage(&[]), None);
    }

    #[test]
    fn reset_reason_parses_every_lowercase_name() {
        for (name, reason) in [
            ("tokens", ResetReason::Tokens),
            ("turns", ResetReason::Turns),
            ("restart", ResetReason::Restart),
        ] {
            let parsed: ResetReason = serde_json::from_str(&format!("\"{name}\"")).unwrap();
            assert_eq!(parsed, reason);
            assert_eq!(
                serde_json::to_string(&reason).unwrap(),
                format!("\"{name}\"")
            );
        }
    }

    #[test]
    fn thread_cell_without_a_live_process_is_cold_at_turn_zero() {
        let data = ThreadUsage {
            reset: Some(ResetReason::Turns),
            ..usage(Some(time(0)))
        };
        let cell = thread_cell(Some(&data), &env(false, Some(9)));
        assert_eq!(
            (cell.turn, cell.ctx_percent, cell.cache_minutes, cell.reset),
            (0, None, None, None)
        );
        assert_eq!(cell.max_turns, 30);
        assert_eq!(cell.shift, Some(9));
    }

    #[test]
    fn thread_cell_for_a_live_process_carries_the_display_data() {
        let data = ThreadUsage {
            write_heavy: true,
            reset: Some(ResetReason::Tokens),
            ..usage(Some(time(-1800)))
        };
        let cell = thread_cell(Some(&data), &env(true, None));
        assert_eq!(
            (cell.turn, cell.ctx_percent, cell.cache_minutes),
            (12, Some(62), Some(30))
        );
        assert!(cell.write_heavy);
        assert_eq!(cell.reset, Some(ResetReason::Tokens));
        assert_eq!(cell.shift, None);
    }

    #[test]
    fn thread_cell_without_usage_or_a_token_limit_has_no_context() {
        let cell = thread_cell(None, &env(true, None));
        assert_eq!(
            (cell.turn, cell.ctx_percent, cell.cache_minutes),
            (0, None, None)
        );
        let data = usage(Some(time(0)));
        let unlimited = ThreadEnv {
            limits: ResetLimits {
                turns: 30,
                tokens: 0,
            },
            ..env(true, None)
        };
        assert_eq!(thread_cell(Some(&data), &unlimited).ctx_percent, None);
    }

    #[test]
    fn thread_variants_run_long_to_short() {
        assert_eq!(
            thread_variants(&cell()),
            [
                "T12/30 · ctx 62% · cache 41m! · shift ~9 msg",
                "T12/30 · 62% · 41m!",
                "12/30 62% 41m!",
                "41m!",
            ]
        );
    }

    #[test]
    fn thread_variants_drop_the_shift_and_the_mark_when_absent() {
        let plain = ThreadCell {
            write_heavy: false,
            shift: None,
            ..cell()
        };
        assert_eq!(
            thread_variants(&plain),
            [
                "T12/30 · ctx 62% · cache 41m",
                "T12/30 · 62% · 41m",
                "12/30 62% 41m",
                "41m"
            ]
        );
    }

    #[test]
    fn thread_variants_show_dashes_and_cold_without_a_process() {
        let idle = ThreadCell {
            turn: 0,
            ctx_percent: None,
            cache_minutes: None,
            write_heavy: true,
            shift: None,
            ..cell()
        };
        assert_eq!(
            thread_variants(&idle),
            [
                "T0/30 · ctx – · cache cold",
                "T0/30 · – · cold",
                "0/30 – cold",
                "cold"
            ]
        );
    }

    #[test]
    fn reset_marks_prefix_only_the_two_long_forms() {
        for (reason, mark) in [
            (ResetReason::Tokens, "↻ tokens · "),
            (ResetReason::Turns, "↻ turns · "),
            (ResetReason::Restart, "↻ restart · "),
        ] {
            let marked = ThreadCell {
                reset: Some(reason),
                shift: None,
                ..cell()
            };
            let forms = thread_variants(&marked);
            assert_eq!(forms[0], format!("{mark}T12/30 · ctx 62% · cache 41m!"));
            assert_eq!(forms[1], format!("{mark}T12/30 · 62% · 41m!"));
            assert_eq!(forms[2], "12/30 62% 41m!");
            assert_eq!(forms[3], "41m!");
        }
    }

    #[test]
    fn cost_variants_show_the_delta_first() {
        let cost = SessionCost {
            total: 4.123,
            delta: Some(0.314),
            missing: false,
        };
        assert_eq!(cost_variants(&cost), ["$4.12 (+0.31)", "$4.12"]);
        let flat = SessionCost {
            delta: None,
            ..cost
        };
        assert_eq!(cost_variants(&flat), ["$4.12"]);
    }
}
