use super::chart::pad_days;
use super::{daily, group_by, parse, Dim, Filter, Group, Report, NO_DATA};
use crate::wire::view::Pricing;
use crate::worker_costs::tickets::parse_windows;
use crate::worker_costs::{count, NO_USAGE};
use chrono::TimeZone;

const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const UNPRICED_MARK: char = '?';
const GAP_MARK: char = ' ';
const GAP: &str = "  ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Daily,
    By(Dim),
    Cache,
}

pub struct Inputs<'a> {
    pub usage: &'a str,
    pub workers: &'a str,
}

fn dollars(group: &Group) -> f64 {
    group.usd().known.unwrap_or(0.0)
}

pub fn sparkline(days: &[Group]) -> String {
    let peak = days.iter().map(dollars).fold(0.0, f64::max);
    days.iter()
        .map(|day| match day.state() {
            Some(NO_DATA) => GAP_MARK,
            Some(_) => UNPRICED_MARK,
            None if peak > 0.0 => LEVELS[(dollars(day) / peak * 7.0).round() as usize],
            None => LEVELS[0],
        })
        .collect()
}

fn dim_name(dim: Dim) -> &'static str {
    match dim {
        Dim::Kind => "kind",
        Dim::Store => "store",
        Dim::Harness => "harness",
        Dim::Model => "model",
        Dim::Ticket => "ticket",
        Dim::Run => "run",
    }
}

fn width(texts: impl Iterator<Item = String>) -> usize {
    texts.map(|text| text.chars().count()).max().unwrap_or(0)
}

fn columns(groups: &[&Group], extra: impl Fn(&Group) -> Option<String>) -> Vec<String> {
    let rows: Vec<&&Group> = groups
        .iter()
        .filter(|group| group.state().is_none())
        .collect();
    let key_width = width(rows.iter().map(|group| group.key.clone()));
    let cost_width = width(rows.iter().map(|group| group.cost_label()));
    groups
        .iter()
        .map(|group| match (group.state(), extra(group)) {
            (None, Some(tail)) => format!(
                "{:<key_width$}{GAP}{:<cost_width$}{GAP}{tail}",
                group.key,
                group.cost_label()
            ),
            (None, None) => format!("{}: {NO_DATA}", group.key),
            (Some(_), _) => group.line(),
        })
        .collect()
}

fn tokens(group: &Group) -> Option<String> {
    let tokens = group.tokens;
    Some(format!(
        "in {}{GAP}out {}",
        count(tokens.input),
        count(tokens.output)
    ))
}

fn page(title: String, body: Vec<String>, malformed: usize) -> String {
    let mut lines = vec![title];
    lines.extend(body);
    if malformed > 0 {
        lines.push(format!("malformed lines skipped: {malformed}"));
    }
    lines.join("\n")
}

fn table(report: &Report, title: String, extra: impl Fn(&Group) -> Option<String>) -> String {
    if report.groups.is_empty() {
        return page(title, vec![NO_USAGE.to_string()], report.malformed);
    }
    let all: Vec<&Group> = report.groups.iter().chain([&report.total]).collect();
    page(title, columns(&all, extra), report.malformed)
}

fn daily_text(report: &Report) -> String {
    let title = "usage daily (local days)".to_string();
    if report.groups.is_empty() {
        return table(report, title, tokens);
    }
    let days = pad_days(&report.groups);
    let used: Vec<&Group> = days
        .iter()
        .filter(|day| day.metered > 0)
        .chain([&report.total])
        .collect();
    let mut body = vec![format!("trend{GAP}{}", sparkline(&days))];
    body.extend(columns(&used, tokens));
    page(title, body, report.malformed)
}

fn cache(group: &Group) -> Option<String> {
    let tokens = group.tokens;
    group.cache_hit_rate().map(|rate| {
        format!(
            "hit {:.0}%{GAP}read {}{GAP}write {}{GAP}input {}",
            rate * 100.0,
            count(tokens.cache_read),
            count(tokens.cache_write),
            count(tokens.input)
        )
    })
}

pub fn render<Tz: TimeZone>(
    view: View,
    inputs: &Inputs,
    filter: &Filter,
    zone: &Tz,
    pricing: &Pricing,
) -> String {
    let windows = parse_windows(inputs.workers);
    let ledger = parse(inputs.usage, &windows);
    match view {
        View::Daily => daily_text(&daily(&ledger, &windows, filter, zone, pricing)),
        View::By(dim) => table(
            &group_by(dim, &ledger, &windows, filter, pricing),
            format!("usage by {}", dim_name(dim)),
            tokens,
        ),
        View::Cache => table(
            &group_by(Dim::Model, &ledger, &windows, filter, pricing),
            "usage cache by model".to_string(),
            cache,
        ),
    }
}

#[cfg(test)]
mod tests;
