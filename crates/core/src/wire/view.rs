use chrono::{DateTime, Utc};
use serde::Serialize;

use super::live::{format_row, hit_fraction, usage_columns};
use super::provider::{CallKind, ProviderId};
use super::record::{CallRecord, SessionRecord};
use super::usage::{CacheWrite, Usage, UsageOutcome};

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
    pub complete: bool,
    pub error: Option<String>,
    #[serde(skip)]
    text: String,
}

impl From<&CallRecord> for CallRow {
    fn from(record: &CallRecord) -> Self {
        let hit_ratio = match &record.usage {
            UsageOutcome::Reported(usage) | UsageOutcome::Partial(usage) => hit_fraction(usage),
            UsageOutcome::Missing => None,
        };
        let mut text = format_row(record);
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
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LsView {
    pub rows: Vec<CallRow>,
    pub totals: Totals,
}

fn add_writes(sum: &mut Vec<CacheWrite>, writes: &[CacheWrite]) {
    for write in writes {
        match sum.iter_mut().find(|entry| entry.tier == write.tier) {
            Some(entry) => entry.tokens += write.tokens,
            None => sum.push(write.clone()),
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
                add_writes(&mut usage.cache_writes, &u.cache_writes);
                equivalent = equivalent.zip(row.input_equivalent).map(|(sum, v)| sum + v);
            }
        }
    }
    Totals {
        counted,
        partial,
        missing,
        hit_ratio: hit_fraction(&usage),
        input_equivalent: if counted == 0 { None } else { equivalent },
        usage,
    }
}

pub fn ls_view(records: &[CallRecord], all: bool) -> LsView {
    let rows: Vec<CallRow> = records
        .iter()
        .filter(|record| all || record.kind == CallKind::Inference)
        .map(CallRow::from)
        .collect();
    let totals = totals(&rows);
    LsView { rows, totals }
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
        "total {} counted ({} partial, {} missing) {} eq={}",
        totals.counted, totals.partial, totals.missing, columns, equivalent
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

pub fn sessions_table(rows: &[SessionRow]) -> String {
    if rows.is_empty() {
        return "no sessions".to_string();
    }
    rows.iter()
        .map(|row| {
            format!(
                "{} {} {:?} {} calls={} inference={}",
                row.id,
                row.started_at.format("%Y-%m-%d %H:%M:%S"),
                row.provider,
                row.upstream,
                row.calls,
                row.inference_calls
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::RequestFacts;
    use crate::wire::record::{RequestBody, SCHEMA_VERSION};
    use crate::wire::usage::CacheTier;
    use chrono::TimeZone;

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
            "id1 2026-10-21 16:24:29 Anthropic http://u calls=2 inference=1"
        );
        assert_eq!(sessions_table(&[]), "no sessions");
    }
}
