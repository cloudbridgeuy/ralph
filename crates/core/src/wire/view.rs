use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::BTreeMap;

use super::live::{format_row, hit_fraction, usage_columns, usd_suffix};
use super::price::{prompt_tokens, resolve_price, usd, PriceOverride, PriceTable};
use super::provider::{CallKind, Provider, ProviderId, RequestFacts, SegmentSource};
use super::read::{Calls, ReadError};
use super::record::{CallRecord, RequestBody, SessionRecord};
use super::usage::{CacheTier, CacheWrite, Usage, UsageOutcome};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallRow {
    pub seq: u64,
    pub ended_at: DateTime<Utc>,
    pub kind: CallKind,
    pub path: String,
    pub model: Option<String>,
    pub status: Option<u16>,
    pub usage: UsageOutcome,
    pub hit_ratio: Option<f64>,
    pub input_equivalent: Option<f64>,
    pub usd: Option<f64>,
    pub complete: bool,
    pub error: Option<String>,
    #[serde(skip)]
    text: String,
}

impl CallRow {
    fn new(record: &CallRecord, usd: Option<f64>) -> Self {
        let hit_ratio = match &record.usage {
            UsageOutcome::Reported(usage) | UsageOutcome::Partial(usage) => hit_fraction(usage),
            UsageOutcome::Missing => None,
        };
        let mut text = format_row(record, usd);
        if matches!(record.usage, UsageOutcome::Partial(_)) {
            text.push_str(" usage=partial");
        }
        if record.kind == CallKind::Other {
            text.push_str(&format!(" other {} {}", record.method, record.path));
        }
        Self {
            seq: record.seq,
            ended_at: record.ended_at,
            kind: record.kind,
            path: record.path.clone(),
            model: record.facts.model.clone(),
            status: record.status,
            usage: record.usage.clone(),
            hit_ratio,
            input_equivalent: record.input_equivalent,
            usd,
            complete: record.complete,
            error: record.error.clone(),
            text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Totals {
    pub counted: usize,
    pub partial: usize,
    pub missing: usize,
    pub usage: Usage,
    pub hit_ratio: Option<f64>,
    pub input_equivalent: Option<f64>,
    pub usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LsView {
    pub rows: Vec<CallRow>,
    pub totals: Totals,
    pub prices_as_of: Option<String>,
}

pub struct Pricing<'a> {
    pub provider: &'a dyn Provider,
    pub overrides: &'a BTreeMap<String, PriceOverride>,
}

fn call_usd(record: &CallRecord, pricing: &Pricing, table: &PriceTable) -> Option<f64> {
    let model = record.facts.model.as_deref()?;
    let (UsageOutcome::Reported(usage) | UsageOutcome::Partial(usage)) = &record.usage else {
        return None;
    };
    let factor = pricing
        .provider
        .price_factor(record.raw_usage.as_ref()?, model)?;
    let price = resolve_price(
        table,
        pricing.overrides,
        &pricing.provider.rates(model),
        model,
        prompt_tokens(usage),
    )?;
    usd(usage, &price).map(|value| value * factor)
}

fn add_writes(sum: &mut Vec<CacheWrite>, writes: &[CacheWrite]) {
    for write in writes {
        match sum.iter_mut().find(|entry| entry.tier == write.tier) {
            Some(entry) => entry.tokens += write.tokens,
            None => sum.push(write.clone()),
        }
    }
}

fn add_other(sum: &mut Vec<(String, u64)>, other: &[(String, u64)]) {
    for (key, tokens) in other {
        match sum.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, total)) => *total += tokens,
            None => sum.push((key.clone(), *tokens)),
        }
    }
}

fn totals(rows: &[CallRow]) -> Totals {
    let mut usage = Usage {
        input_uncached: 0,
        cache_read: 0,
        cache_writes: vec![],
        output: 0,
        reasoning: None,
        other: vec![],
    };
    let mut counted = 0;
    let mut partial = 0;
    let mut missing = 0;
    let mut equivalent = Some(0.0);
    let mut cost = Some(0.0);
    for row in rows.iter().filter(|row| row.kind == CallKind::Inference) {
        match &row.usage {
            UsageOutcome::Missing => missing += 1,
            UsageOutcome::Reported(u) | UsageOutcome::Partial(u) => {
                if matches!(row.usage, UsageOutcome::Partial(_)) {
                    partial += 1;
                }
                counted += 1;
                usage.input_uncached += u.input_uncached;
                usage.cache_read += u.cache_read;
                usage.output += u.output;
                usage.reasoning = match (usage.reasoning, u.reasoning) {
                    (Some(sum), Some(v)) => Some(sum + v),
                    (sum, v) => sum.or(v),
                };
                add_writes(&mut usage.cache_writes, &u.cache_writes);
                add_other(&mut usage.other, &u.other);
                equivalent = equivalent.zip(row.input_equivalent).map(|(sum, v)| sum + v);
                cost = cost.zip(row.usd).map(|(sum, v)| sum + v);
            }
        }
    }
    Totals {
        counted,
        partial,
        missing,
        hit_ratio: hit_fraction(&usage),
        input_equivalent: if counted == 0 { None } else { equivalent },
        usd: if counted == 0 { None } else { cost },
        usage,
    }
}

pub fn ls_view(records: &[CallRecord], all: bool, pricing: &Pricing) -> LsView {
    let table = pricing.provider.prices();
    let rows: Vec<CallRow> = records
        .iter()
        .filter(|record| all || record.kind == CallKind::Inference)
        .map(|record| CallRow::new(record, call_usd(record, pricing, &table)))
        .collect();
    let totals = totals(&rows);
    let prices_as_of = rows
        .iter()
        .any(|row| row.usd.is_some())
        .then(|| table.as_of.to_string());
    LsView {
        rows,
        totals,
        prices_as_of,
    }
}

pub fn ls_table(view: &LsView) -> String {
    let totals = &view.totals;
    let mut lines: Vec<String> = view.rows.iter().map(|row| row.text.clone()).collect();
    let equivalent = totals
        .input_equivalent
        .map_or_else(|| "-".to_string(), |value| format!("{value:.0}"));
    let columns = if totals.counted == 0 {
        "usage=none".to_string()
    } else {
        usage_columns(&UsageOutcome::Reported(totals.usage.clone()))
    };
    lines.push(format!(
        "total {} counted ({} partial, {} missing) {} eq={}{}",
        totals.counted,
        totals.partial,
        totals.missing,
        columns,
        equivalent,
        usd_suffix(totals.usd)
    ));
    lines.join("\n")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionRow {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub provider: ProviderId,
    pub upstream: String,
    pub calls: usize,
    pub inference_calls: usize,
}

pub fn session_row(id: &str, session: &SessionRecord, calls: &[CallRecord]) -> SessionRow {
    SessionRow {
        id: id.to_string(),
        started_at: session.started_at,
        provider: session.provider,
        upstream: session.upstream.clone(),
        calls: calls.len(),
        inference_calls: calls
            .iter()
            .filter(|call| call.kind == CallKind::Inference)
            .count(),
    }
}

pub type SessionRead = (String, Result<(SessionRecord, Calls), ReadError>);

pub fn session_rows(reads: Vec<SessionRead>) -> (Vec<SessionRow>, Vec<String>) {
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    for (id, read) in reads {
        match read {
            Ok((session, calls)) => rows.push(session_row(&id, &session, &calls.records)),
            Err(error) => warnings.push(format!("skipped {id}: {error}")),
        }
    }
    (rows, warnings)
}

pub fn sessions_table(rows: &[SessionRow]) -> String {
    if rows.is_empty() {
        return "no sessions".to_string();
    }
    rows.iter()
        .map(|row| {
            format!(
                "{} {} {} {} calls={} inference={}",
                row.id,
                row.started_at.format("%Y-%m-%d %H:%M:%S"),
                row.provider.as_str(),
                row.upstream,
                row.calls,
                row.inference_calls
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SegmentRow {
    pub label: String,
    pub source: SegmentSource,
    pub bytes: usize,
    pub cumulative: usize,
    pub breakpoint: Option<CacheTier>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShowView {
    pub session: String,
    pub seq: u64,
    pub kind: CallKind,
    pub method: String,
    pub path: String,
    pub status: Option<u16>,
    pub body_is_json: bool,
    pub segments: Vec<SegmentRow>,
    pub facts: RequestFacts,
    pub usage: UsageOutcome,
    pub raw_usage: Option<serde_json::Value>,
}

pub fn show_view(record: &CallRecord, session: &str, provider: &dyn Provider) -> ShowView {
    let (body_is_json, prompt) = match &record.request_body {
        RequestBody::Json(body) => (true, provider.segments(body)),
        RequestBody::Base64(_) => (false, Vec::new()),
    };
    let mut cumulative = 0;
    let segments = prompt
        .into_iter()
        .map(|segment| {
            cumulative += segment.text.len();
            SegmentRow {
                label: segment.label,
                source: segment.source,
                bytes: segment.text.len(),
                cumulative,
                breakpoint: segment.breakpoint,
            }
        })
        .collect();
    ShowView {
        session: session.to_string(),
        seq: record.seq,
        kind: record.kind,
        method: record.method.clone(),
        path: record.path.clone(),
        status: record.status,
        body_is_json,
        segments,
        facts: record.facts.clone(),
        usage: record.usage.clone(),
        raw_usage: record.raw_usage.clone(),
    }
}

fn tier_text(tier: CacheTier) -> String {
    match tier {
        CacheTier::Ttl { seconds } if seconds % 3600 == 0 => format!("{}h", seconds / 3600),
        CacheTier::Ttl { seconds } if seconds % 60 == 0 => format!("{}m", seconds / 60),
        CacheTier::Ttl { seconds } => format!("{seconds}s"),
        CacheTier::Unspecified => "?".to_string(),
    }
}

fn json_text<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| format!("<{error}>"))
}

pub fn show_table(view: &ShowView) -> String {
    let other = if view.kind == CallKind::Other {
        "other "
    } else {
        ""
    };
    let status = view
        .status
        .map_or_else(|| "-".to_string(), |s| s.to_string());
    let mut lines = vec![
        format!("session: {}", view.session),
        format!("call {}", view.seq),
        format!("{other}{} {} status {status}", view.method, view.path),
    ];
    if view.body_is_json && view.segments.is_empty() {
        lines.push("no segments".to_string());
    } else if view.body_is_json {
        lines.push("segments (bytes approximate, re-encoded from parsed JSON):".to_string());
        lines.extend(view.segments.iter().map(|row| {
            let breakpoint = row.breakpoint.map_or(String::new(), |tier| {
                format!(" breakpoint={}", tier_text(tier))
            });
            format!(
                "  {} bytes={} cumulative={}{}",
                row.label, row.bytes, row.cumulative, breakpoint
            )
        }));
    } else {
        lines.push("request body is not JSON; no segments".to_string());
    }
    lines.push(format!(
        "model: {}",
        view.facts.model.as_deref().unwrap_or("-")
    ));
    lines.push(format!("params: {}", json_text(&view.facts.params)));
    lines.push(format!("usage: {}", json_text(&view.usage)));
    lines.push(format!(
        "raw_usage: {}",
        view.raw_usage
            .as_ref()
            .map_or_else(|| "-".to_string(), json_text)
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::anthropic::Anthropic;
    use crate::wire::provider::RequestFacts;
    use crate::wire::record::{RequestBody, SCHEMA_VERSION};
    use crate::wire::usage::CacheTier;
    use chrono::TimeZone;
    use serde_json::json;

    fn ls_view(records: &[CallRecord], all: bool) -> LsView {
        super::ls_view(
            records,
            all,
            &Pricing {
                provider: &Anthropic,
                overrides: &BTreeMap::new(),
            },
        )
    }

    fn priced(seq: u64, usage: Usage, raw: serde_json::Value) -> CallRecord {
        let mut call = record(seq, UsageOutcome::Reported(usage), Some(1.0));
        call.facts.model = Some("claude-haiku-5-5".into());
        call.raw_usage = Some(raw);
        call
    }

    fn usage(uncached: u64, read: u64, five_min: u64, out: u64) -> Usage {
        Usage {
            input_uncached: uncached,
            cache_read: read,
            cache_writes: vec![CacheWrite {
                tier: CacheTier::Ttl { seconds: 300 },
                tokens: five_min,
            }],
            output: out,
            reasoning: None,
            other: vec![],
        }
    }

    fn record(seq: u64, usage: UsageOutcome, eq: Option<f64>) -> CallRecord {
        let at = Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap();
        CallRecord {
            seq,
            started_at: at,
            first_byte_at: None,
            ended_at: at,
            method: "POST".into(),
            path: "/v1/messages".into(),
            status: Some(200),
            kind: CallKind::Inference,
            facts: RequestFacts {
                model: Some("m".into()),
                params: serde_json::Map::new(),
            },
            request_headers: vec![],
            response_headers: vec![],
            request_body: RequestBody::Base64(String::new()),
            raw_usage: None,
            usage,
            input_equivalent: eq,
            complete: true,
            error: None,
            response_events: None,
        }
    }

    fn other(seq: u64) -> CallRecord {
        let mut call = record(seq, UsageOutcome::Missing, None);
        call.kind = CallKind::Other;
        call.method = "GET".into();
        call.path = "/v1/models".into();
        call
    }

    #[test]
    fn totals_skip_missing_and_count_partial() {
        let records = vec![
            record(1, UsageOutcome::Reported(usage(10, 90, 0, 5)), Some(100.0)),
            record(2, UsageOutcome::Partial(usage(0, 0, 100, 1)), Some(20.0)),
            record(3, UsageOutcome::Missing, None),
        ];
        let view = ls_view(&records, false);
        let totals = &view.totals;
        assert_eq!((totals.counted, totals.partial, totals.missing), (2, 1, 1));
        assert_eq!(totals.usage.input_uncached, 10);
        assert_eq!(totals.usage.cache_read, 90);
        assert_eq!(totals.usage.cache_writes[0].tokens, 100);
        assert_eq!(totals.usage.output, 6);
        assert_eq!(totals.input_equivalent, Some(120.0));
        assert_eq!(totals.hit_ratio, Some(0.45));
        let table = ls_table(&view);
        assert!(table.contains("usage=missing"), "{table}");
        assert!(
            table.contains("w5m=100 out=1 hit=0% eq=20 usage=partial"),
            "{table}"
        );
        assert!(
            table.ends_with(
                "total 2 counted (1 partial, 1 missing) in=10 cr=90 w5m=100 out=6 hit=45% eq=120"
            ),
            "{table}"
        );
    }

    #[test]
    fn totals_sum_reasoning_when_any_row_reports_it() {
        let mut with = usage(1, 0, 0, 1);
        with.reasoning = Some(7);
        let records = vec![
            record(1, UsageOutcome::Reported(with), None),
            record(2, UsageOutcome::Reported(usage(1, 0, 0, 1)), None),
        ];
        assert_eq!(ls_view(&records, false).totals.usage.reasoning, Some(7));
        let none = vec![record(1, UsageOutcome::Reported(usage(1, 0, 0, 1)), None)];
        assert_eq!(ls_view(&none, false).totals.usage.reasoning, None);
    }

    #[test]
    fn totals_merge_other_keys_in_first_seen_order() {
        let mut first = usage(1, 0, 0, 1);
        first.other = vec![("a".into(), 1), ("b".into(), 2)];
        let mut second = usage(1, 0, 0, 1);
        second.other = vec![("c".into(), 5), ("a".into(), 4)];
        let records = vec![
            record(1, UsageOutcome::Reported(first), None),
            record(2, UsageOutcome::Partial(second), None),
        ];
        assert_eq!(
            ls_view(&records, false).totals.usage.other,
            vec![
                ("a".to_string(), 5),
                ("b".to_string(), 2),
                ("c".to_string(), 5)
            ]
        );
    }

    #[test]
    fn unrated_row_makes_total_cost_unknown() {
        let records = vec![
            record(1, UsageOutcome::Reported(usage(1, 0, 0, 1)), Some(2.0)),
            record(2, UsageOutcome::Reported(usage(1, 0, 0, 1)), None),
        ];
        let view = ls_view(&records, false);
        assert_eq!(view.totals.input_equivalent, None);
        assert!(ls_table(&view).ends_with("eq=-"));
        let json = serde_json::to_value(&view).unwrap();
        assert!(json["totals"]["input_equivalent"].is_null());
    }

    #[test]
    fn no_counted_usage_shows_none_not_zeros() {
        let view = ls_view(&[record(1, UsageOutcome::Missing, None)], false);
        assert_eq!(
            ls_table(&view).lines().last().unwrap(),
            "total 0 counted (0 partial, 1 missing) usage=none eq=-"
        );
    }

    #[test]
    fn all_flag_includes_other_calls_but_not_in_totals() {
        let records = vec![
            record(1, UsageOutcome::Reported(usage(1, 0, 0, 1)), Some(2.0)),
            other(2),
        ];
        assert_eq!(ls_view(&records, false).rows.len(), 1);
        let view = ls_view(&records, true);
        assert_eq!(view.rows.len(), 2);
        assert_eq!(view.totals.missing, 0);
        assert!(ls_table(&view).contains("other GET /v1/models"));
    }

    #[test]
    fn json_row_carries_full_usage_outcome() {
        let mut u = usage(1, 2, 3, 4);
        u.reasoning = Some(9);
        let view = ls_view(&[record(1, UsageOutcome::Partial(u), None)], false);
        let row = serde_json::to_value(&view.rows[0]).unwrap();
        assert_eq!(row["usage"]["status"], "partial");
        assert_eq!(row["usage"]["usage"]["reasoning"], 9);
        assert!(row.get("text").is_none());
    }

    #[test]
    fn sessions_rows_and_table() {
        let session = SessionRecord {
            schema_version: SCHEMA_VERSION,
            ralph_version: "0".into(),
            provider: ProviderId::Anthropic,
            upstream: "http://u".into(),
            listen: "127.0.0.1:1".into(),
            started_at: Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap(),
        };
        let calls = vec![record(1, UsageOutcome::Missing, None), other(2)];
        let row = session_row("id1", &session, &calls);
        assert_eq!((row.calls, row.inference_calls), (2, 1));
        assert_eq!(
            sessions_table(&[row]),
            "id1 2026-10-21 16:24:29 anthropic http://u calls=2 inference=1"
        );
        assert_eq!(sessions_table(&[]), "no sessions");
    }

    #[test]
    fn damaged_sessions_are_skipped_with_a_warning() {
        let session = SessionRecord {
            schema_version: SCHEMA_VERSION,
            ralph_version: "0".into(),
            provider: ProviderId::Anthropic,
            upstream: "http://u".into(),
            listen: "127.0.0.1:1".into(),
            started_at: Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap(),
        };
        let bad = ReadError::Session {
            label: "/r/b/session.json".into(),
            message: "EOF".into(),
        };
        let full = Calls {
            records: vec![record(1, UsageOutcome::Missing, None)],
            torn_tail: false,
        };
        let (rows, warnings) = session_rows(vec![
            ("a".into(), Ok((session.clone(), full))),
            ("b".into(), Err(bad)),
            ("c".into(), Ok((session, Calls::default()))),
        ]);
        assert_eq!(
            rows.iter()
                .map(|r| (r.id.as_str(), r.calls))
                .collect::<Vec<_>>(),
            vec![("a", 1), ("c", 0)]
        );
        assert_eq!(warnings, vec!["skipped b: /r/b/session.json: EOF"]);
    }

    #[test]
    fn priced_rows_show_usd_in_rows_total_and_json() {
        let records = vec![
            priced(1, usage(100_000, 0, 0, 0), json!({})),
            priced(2, usage(0, 0, 0, 1_000_000), json!({"inference_geo": "us"})),
        ];
        let view = ls_view(&records, false);
        assert!((view.rows[0].usd.unwrap() - 0.01).abs() < 1e-9);
        assert!((view.rows[1].usd.unwrap() - 0.55).abs() < 1e-9);
        assert!((view.totals.usd.unwrap() - 0.56).abs() < 1e-9);
        assert_eq!(view.prices_as_of.as_deref(), Some("2026-10-21"));
        let table = ls_table(&view);
        assert!(table.contains("eq=1 $=0.010000"), "{table}");
        assert!(table.ends_with("eq=2 $=0.560000"), "{table}");
        let json = serde_json::to_value(&view).unwrap();
        assert!((json["rows"][0]["usd"].as_f64().unwrap() - 0.01).abs() < 1e-9);
        assert_eq!(json["prices_as_of"], "2026-10-21");
    }

    #[test]
    fn unpriced_row_hides_usd_and_nulls_total() {
        let records = vec![
            priced(1, usage(10, 0, 0, 0), json!({})),
            priced(2, usage(10, 0, 0, 0), json!({"speed": "fast"})),
        ];
        let view = ls_view(&records, false);
        assert_eq!(view.rows[1].usd, None);
        assert_eq!(view.totals.usd, None);
        let table = ls_table(&view);
        assert!(!table.lines().nth(1).unwrap().contains('$'), "{table}");
        assert!(!table.lines().last().unwrap().contains('$'), "{table}");
        let json = serde_json::to_value(&view).unwrap();
        assert!(json["totals"]["usd"].is_null());
        assert!(json["rows"][1]["usd"].is_null());
    }

    #[test]
    fn no_price_inputs_leaves_prices_as_of_null() {
        let view = ls_view(&[record(1, UsageOutcome::Missing, None)], false);
        assert_eq!(view.prices_as_of, None);
        assert!(serde_json::to_value(&view).unwrap()["prices_as_of"].is_null());
    }

    #[test]
    fn override_prices_unknown_model() {
        let overrides = BTreeMap::from([(
            "m".to_string(),
            PriceOverride {
                input: 2.0,
                cache_read: None,
                cache_write_5m: None,
                cache_write_1h: None,
                output: None,
            },
        )]);
        let mut call = record(1, UsageOutcome::Reported(usage(500_000, 0, 0, 0)), None);
        call.raw_usage = Some(json!({}));
        let pricing = Pricing {
            provider: &Anthropic,
            overrides: &overrides,
        };
        let view = super::ls_view(&[call], false, &pricing);
        assert_eq!(view.rows[0].usd, Some(1.0));
    }

    fn json_call() -> CallRecord {
        let mut call = record(7, UsageOutcome::Missing, None);
        call.request_body = RequestBody::Json(json!({
            "tools": [{"name": "Read", "cache_control": {"type": "ephemeral", "ttl": "1h"}}],
            "system": "sys",
        }));
        call
    }

    #[test]
    fn show_view_accumulates_bytes_and_keeps_breakpoints() {
        let view = show_view(&json_call(), "s1", &Anthropic);
        assert_eq!(view.seq, 7);
        let rows: Vec<(&str, usize, usize, Option<CacheTier>)> = view
            .segments
            .iter()
            .map(|r| (r.label.as_str(), r.bytes, r.cumulative, r.breakpoint))
            .collect();
        let tool = json!({"name": "Read", "cache_control": {"type": "ephemeral", "ttl": "1h"}})
            .to_string()
            .len();
        assert_eq!(
            rows,
            [
                (
                    "tools[0] Read",
                    tool,
                    tool,
                    Some(CacheTier::Ttl { seconds: 3600 })
                ),
                ("system[0]", 28, tool + 28, None)
            ]
        );
    }

    #[test]
    fn show_table_lists_segments_facts_and_usage() {
        let table = show_table(&show_view(&json_call(), "s1", &Anthropic));
        assert!(
            table.starts_with("session: s1\ncall 7\nPOST /v1/messages status 200\n"),
            "{table}"
        );
        assert!(table.contains("  tools[0] Read bytes="), "{table}");
        assert!(table.contains(" breakpoint=1h\n"), "{table}");
        assert!(
            table.contains("  system[0] bytes=28 cumulative="),
            "{table}"
        );
        assert!(table.contains("model: m\n"), "{table}");
        assert!(
            table.contains("usage: {\"status\":\"missing\"}\n"),
            "{table}"
        );
        assert!(table.ends_with("raw_usage: -"), "{table}");
    }

    #[test]
    fn base64_body_has_no_segments_and_says_so() {
        let view = show_view(&record(1, UsageOutcome::Missing, None), "s1", &Anthropic);
        assert!(!view.body_is_json);
        assert!(view.segments.is_empty());
        assert!(show_table(&view).contains("request body is not JSON"));
    }

    #[test]
    fn show_table_names_other_call_method_path_and_status() {
        let mut call = other(3);
        call.status = None;
        let table = show_table(&show_view(&call, "s1", &Anthropic));
        assert_eq!(
            table,
            "session: s1\ncall 3\nother GET /v1/models status -\n\
             request body is not JSON; no segments\n\
             model: m\nparams: {}\nusage: {\"status\":\"missing\"}\nraw_usage: -"
        );
    }

    #[test]
    fn show_view_json_carries_session_and_call_fields() {
        let json = serde_json::to_value(show_view(&json_call(), "s1", &Anthropic)).unwrap();
        assert_eq!(json["session"], "s1");
        assert_eq!(json["seq"], 7);
        assert_eq!(json["kind"], "inference");
        assert_eq!(json["method"], "POST");
        assert_eq!(json["path"], "/v1/messages");
        assert_eq!(json["status"], 200);
        assert_eq!(json["segments"][0]["source"], "tools");
        assert_eq!(json["segments"][1]["source"], "system");
    }

    #[test]
    fn json_body_without_segments_says_no_segments() {
        let mut call = record(2, UsageOutcome::Missing, None);
        call.request_body = RequestBody::Json(json!({}));
        let table = show_table(&show_view(&call, "s1", &Anthropic));
        assert!(table.contains("status 200\nno segments\nmodel:"), "{table}");
        assert!(!table.contains("segments (bytes"), "{table}");
    }
}
