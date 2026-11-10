use super::tickets::parse_windows;
use super::usage_report::{group_by, parse, Dim, Filter, Group};
use super::{costs_view, count, tier, Usd, MISSING, NO_USAGE, UNKNOWN};
use crate::wire::view::Pricing;
use chrono::{DateTime, Utc};

const HEADERS: [&str; 8] = [
    "INPUT",
    "OUTPUT",
    "CACHE READ",
    "CACHE WRITE",
    "5M",
    "1H",
    "REASONING",
    "USD",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    HarnessModel,
    Harness,
    Role,
    Ticket,
}

pub struct Inputs<'a> {
    pub usage: &'a str,
    pub workers: &'a str,
}

fn cells(group: &Group) -> Vec<String> {
    let mut cells = vec![group.key.clone()];
    if group.metered == 0 {
        cells.extend(HEADERS.map(|_| UNKNOWN.to_string()));
        return cells;
    }
    let partial = group.unmetered > 0;
    let mark = |text: String| {
        if partial && text != MISSING && !text.ends_with('+') {
            format!("{text}+")
        } else {
            text
        }
    };
    let tokens = group.tokens;
    let usd = Usd {
        missing: group.usd().missing || partial,
        ..group.usd()
    };
    cells.extend(
        [
            count(tokens.input),
            count(tokens.output),
            count(tokens.cache_read),
            count(tokens.cache_write),
            tier(tokens.write_5m, tokens.partial_5m),
            tier(tokens.write_1h, tokens.partial_1h),
            tier(tokens.reasoning, tokens.partial_reasoning),
        ]
        .map(mark),
    );
    cells.push(group.with_excluded(usd.text()));
    cells
}

fn key_header(dim: Dim) -> &'static str {
    match dim {
        Dim::Harness => "HARNESS",
        Dim::Ticket => "TICKET",
        _ => "ROLE",
    }
}

fn markdown(dim: Dim, groups: &[Group], total: &Group) -> String {
    let line = |cells: &[String]| format!("| {} |", cells.join(" | "));
    let headers: Vec<String> = std::iter::once(key_header(dim))
        .chain(HEADERS)
        .map(String::from)
        .collect();
    let mut lines = vec![
        line(&headers),
        line(&vec!["---".to_string(); headers.len()]),
    ];
    lines.extend(groups.iter().map(|group| line(&cells(group))));
    lines.push(line(&cells(total)));
    lines.join("\n")
}

pub fn costs_by_view(
    view: View,
    since: Option<DateTime<Utc>>,
    inputs: &Inputs,
    pricing: &Pricing,
) -> String {
    let dim = match view {
        View::HarnessModel => return costs_view(inputs.usage, since, pricing),
        View::Harness => Dim::Harness,
        View::Role => Dim::Kind,
        View::Ticket => Dim::Ticket,
    };
    let windows = parse_windows(inputs.workers);
    let filter = Filter {
        since,
        ..Filter::default()
    };
    let report = group_by(
        dim,
        &parse(inputs.usage, &windows),
        &windows,
        &filter,
        pricing,
    );
    if report.groups.is_empty() {
        return NO_USAGE.to_string();
    }
    markdown(dim, &report.groups, &report.total)
}

#[cfg(test)]
mod tests;
