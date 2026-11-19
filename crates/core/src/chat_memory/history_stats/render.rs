use super::counts::{Counts, ToolCounts};
use super::opened::ZoomStats;
use super::parse::Tool;
use super::stats::Stats;

fn tool_label(tool: Tool) -> &'static str {
    match tool {
        Tool::Read => "read",
        Tool::Zoom => "zoom",
        Tool::Recall => "recall",
        Tool::Search => "search",
        Tool::Unknown => "other",
    }
}

fn calls_section(counts: &Counts) -> Vec<String> {
    let mut lines = vec![
        "calls".to_string(),
        format!(
            "{:<8}{:>6}{:>6}{:>6}{:>6}{:>6}",
            "tool", "total", "mcp", "slash", "cli", "other"
        ),
    ];
    lines.extend(counts.tools.iter().map(|row| {
        format!(
            "{:<8}{:>6}{:>6}{:>6}{:>6}{:>6}",
            tool_label(row.tool),
            row.total,
            row.mcp,
            row.slash,
            row.cli,
            row.other_source
        )
    }));
    lines
}

fn rate_row(row: &ToolCounts) -> String {
    format!(
        "{:<8}{:>5}%{:>5}%",
        tool_label(row.tool),
        row.empty_pct(),
        row.error_pct()
    )
}

fn rates_section(counts: &Counts) -> Vec<String> {
    let mut lines = vec![
        "rates".to_string(),
        format!("{:<8}{:>6}{:>6}", "tool", "empty", "error"),
    ];
    lines.extend(counts.tools.iter().map(rate_row));
    lines
}

fn turns_section(stats: &Stats) -> Vec<String> {
    let join = &stats.join;
    let mut lines = vec!["no-history turns".to_string()];
    if join.timing_present {
        lines.push(format!("turns: {}", join.total_turns));
        lines.push(format!("joined: {}", join.joined));
        lines.push(format!("with no history call: {}", join.no_history_calls));
        lines.push(format!(
            "timing rows with no history row: {}",
            join.unjoined_timing
        ));
        lines.push(format!(
            "history rows with no timing row: {}",
            join.history_without_timing
        ));
    } else {
        lines.push(format!("turns: {} (no timing log)", join.total_turns));
        lines.push("joined: n/a".to_string());
        lines.push(format!("with no history call: {}", join.no_history_calls));
    }
    lines
}

fn depth_section(zoom: &ZoomStats) -> Vec<String> {
    let mut lines = vec!["zoom depth".to_string()];
    for (depth, count) in zoom.depth.iter().enumerate() {
        let label = if depth == zoom.depth.len() - 1 {
            format!("{depth}+")
        } else {
            depth.to_string()
        };
        lines.push(format!("{label}: {count}"));
    }
    lines.push(format!(
        "node size: count {}, max {}",
        zoom.size_count, zoom.size_max
    ));
    lines
}

pub fn render_stats(stats: &Stats) -> String {
    if stats.call_rows == 0 && stats.turn_rows == 0 && stats.timing_rows == 0 {
        return "no history rows\n".to_string();
    }
    let sections = [
        vec![
            "history stats".to_string(),
            format!(
                "call rows: {}, turn rows: {}, skipped rows: {}",
                stats.call_rows, stats.turn_rows, stats.skipped
            ),
        ],
        calls_section(&stats.counts),
        rates_section(&stats.counts),
        vec![
            "chains".to_string(),
            format!("chains: {}", stats.chains.chains),
            format!("calls in chains: {}", stats.chains.calls_in_chains),
        ],
        vec![
            "unopened hits".to_string(),
            format!("opened: {}", stats.opened.opened),
            format!("unopened: {}", stats.opened.unopened),
            format!("unknown: {}", stats.opened.unknown),
        ],
        turns_section(stats),
        depth_section(&stats.zoom),
        vec![
            "old reach".to_string(),
            format!("old: {}", stats.old.old),
            format!("decided: {}", stats.old.decided),
            format!("undecided: {}", stats.old.undecided),
        ],
    ];
    let blocks: Vec<String> = sections.iter().map(|lines| lines.join("\n")).collect();
    format!("{}\n", blocks.join("\n\n"))
}
