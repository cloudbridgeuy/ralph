use super::cache_periods::{
    idle_periods, miss_causes, outcome_counts, reset_writes, MissCauses, OutcomeCounts, ResetWrites,
};
use super::cache_saving::{saving_estimate, saving_lines, SavingRates};
use super::cache_view::{
    process_costs, runless_rows, summarize, ProcessStart, ProcessSummary, START_ORDER,
};
use super::{Entry, Role};
use crate::worker_costs::count;

const NONE: &str = "-";

fn money(value: Option<f64>) -> String {
    value.map_or_else(|| NONE.to_string(), |value| format!("${value:.2}"))
}

fn start_label(start: ProcessStart) -> &'static str {
    START_ORDER
        .iter()
        .find(|(_, candidate)| *candidate == start)
        .map_or("cut", |(label, _)| label)
}

pub fn process_lines(summary: &ProcessSummary, cut: usize) -> Vec<String> {
    let turns = summary
        .median_turns
        .map_or_else(|| NONE.to_string(), |turns| turns.to_string());
    let mut lines = vec![format!(
        "{} processes  total ${:.2}  mean {}  median {}  median turns {turns}",
        summary.processes,
        summary.total,
        money(summary.mean),
        money(summary.median)
    )];
    lines.extend(
        summary.by_start.iter().map(|(label, processes, cost)| {
            format!("  {label}  {processes} processes  ${cost:.2}")
        }),
    );
    if cut > 0 {
        lines.push(format!(
            "{cut} process(es) cut by the window: cost counted, no turn stats"
        ));
    }
    lines
}

pub fn period_lines(counts: &OutcomeCounts) -> Vec<String> {
    vec![
        format!(
            "{} periods  {} pings  ${:.2} ping cost",
            counts.periods, counts.pings, counts.ping_cost
        ),
        format!(
            "next turn warm {}  cold {}  no next turn {}",
            counts.warm, counts.cold, counts.no_next
        ),
    ]
}

pub fn reset_lines(groups: &[ResetWrites]) -> Vec<String> {
    groups
        .iter()
        .map(|group| {
            let ratio = group.mean_ratio.map_or_else(
                || "no ratio".to_string(),
                |ratio| format!("writes {:.0}% of context", ratio * 100.0),
            );
            let mean = group.mean_write_tokens.unwrap_or(0.0).round() as u64;
            format!(
                "{}  {} turns  {ratio}  (mean {} tokens)",
                start_label(group.start),
                group.turns,
                count(mean)
            )
        })
        .collect()
}

pub fn skipped_lines(count: usize) -> Vec<String> {
    let noun = if count == 1 { "row" } else { "rows" };
    (count > 0)
        .then(|| format!("{count} main {noun} without a run skipped (no chat to join them to)"))
        .into_iter()
        .collect()
}

pub fn miss_lines(causes: &MissCauses) -> Vec<String> {
    vec![format!(
        "{} misses in {} turns: reset {}  ttl expiry {}  unexplained {}  new run {}",
        causes.misses, causes.turns, causes.reset, causes.ttl, causes.unexplained, causes.new_run
    )]
}

fn section(heading: &str, body: Vec<String>) -> Vec<String> {
    let mut lines = vec![String::new(), heading.to_string()];
    lines.extend(body);
    lines
}

pub fn sections(entries: &[Entry], rates: impl Fn(&str) -> Option<SavingRates>) -> Vec<String> {
    if !entries.iter().any(|entry| entry.role == Role::Main) {
        return Vec::new();
    }
    let costs = process_costs(entries);
    let cut = costs
        .iter()
        .filter(|cost| cost.start == ProcessStart::Cut)
        .count();
    let periods = idle_periods(entries);
    [
        section(
            "cost per claude process",
            process_lines(&summarize(&costs), cut),
        ),
        section(
            "keepalive per idle period",
            period_lines(&outcome_counts(&periods)),
        ),
        section(
            "first turn after a reset",
            reset_lines(&reset_writes(entries)),
        ),
        section("cache misses", miss_lines(&miss_causes(entries))),
        [
            vec![String::new()],
            saving_lines(&saving_estimate(&periods, rates)),
        ]
        .concat(),
        skipped_lines(runless_rows(entries)),
    ]
    .concat()
}

#[cfg(test)]
#[path = "cache_text_tests.rs"]
mod cache_text_tests;
