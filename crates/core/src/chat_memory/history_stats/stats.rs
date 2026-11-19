use super::chains::{build_chains, Chains};
use super::config::StatsConfig;
use super::counts::{count_calls, Counts};
use super::join::{join_turns, Join};
use super::old::{count_old, OldReach};
use super::opened::{count_opened, zoom_stats, Opened, ZoomStats};
use super::parse::{parse_history, parse_timing};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub call_rows: usize,
    pub turn_rows: usize,
    pub timing_rows: usize,
    pub skipped: usize,
    pub counts: Counts,
    pub chains: Chains,
    pub opened: Opened,
    pub zoom: ZoomStats,
    pub old: OldReach,
    pub join: Join,
}

pub fn build_stats(history_text: &str, timing_text: &str, cfg: &StatsConfig) -> Stats {
    let history = parse_history(history_text);
    let timing = parse_timing(timing_text);
    Stats {
        call_rows: history.calls.len(),
        turn_rows: history.turns.len(),
        timing_rows: timing.turns.len(),
        skipped: history.skipped + timing.skipped,
        counts: count_calls(&history.calls),
        chains: build_chains(&history.calls, cfg),
        opened: count_opened(&history.calls),
        zoom: zoom_stats(&history.calls),
        old: count_old(&history.calls, &history.turns),
        join: join_turns(&history.turns, &timing.turns, cfg),
    }
}
