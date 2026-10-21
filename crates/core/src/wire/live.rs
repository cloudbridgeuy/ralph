use std::fmt::Write;

use super::provider::CallKind;
use super::record::CallRecord;
use super::usage::{CacheTier, CacheWrite, Usage, UsageOutcome};

fn tier_label(tier: CacheTier) -> String {
    match tier {
        CacheTier::Unspecified => "w?".to_string(),
        CacheTier::Ttl { seconds } if seconds % 3600 == 0 => format!("w{}h", seconds / 3600),
        CacheTier::Ttl { seconds } if seconds % 60 == 0 => format!("w{}m", seconds / 60),
        CacheTier::Ttl { seconds } => format!("w{seconds}s"),
    }
}

fn writes_label(writes: &[CacheWrite]) -> String {
    if writes.is_empty() {
        return "w=0".to_string();
    }
    writes
        .iter()
        .map(|write| format!("{}={}", tier_label(write.tier), write.tokens))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn hit_fraction(usage: &Usage) -> Option<f64> {
    let written: u64 = usage.cache_writes.iter().map(|write| write.tokens).sum();
    let total = usage.input_uncached + usage.cache_read + written;
    (total != 0).then(|| usage.cache_read as f64 / total as f64)
}

fn hit_ratio(usage: &Usage) -> String {
    hit_fraction(usage).map_or_else(
        || "-".to_string(),
        |fraction| format!("{:.0}%", fraction * 100.0),
    )
}

pub(super) fn usage_columns(outcome: &UsageOutcome) -> String {
    match outcome {
        UsageOutcome::Reported(usage) | UsageOutcome::Partial(usage) => format!(
            "in={} cr={} {} out={} hit={}",
            usage.input_uncached,
            usage.cache_read,
            writes_label(&usage.cache_writes),
            usage.output,
            hit_ratio(usage)
        ),
        UsageOutcome::Missing => "usage=missing".to_string(),
    }
}

pub(super) fn usd_suffix(usd: Option<f64>) -> String {
    usd.map_or_else(String::new, |value| format!(" $={value:.6}"))
}

pub(super) fn format_row(record: &CallRecord, usd: Option<f64>) -> String {
    let status = record
        .status
        .map_or_else(|| "-".to_string(), |status| status.to_string());
    let equivalent = record
        .input_equivalent
        .map_or_else(|| "-".to_string(), |value| format!("{value:.0}"));
    let mut row = format!(
        "#{} {} {} {} {} eq={}{}",
        record.seq,
        record.ended_at.format("%H:%M:%S"),
        record.facts.model.as_deref().unwrap_or("-"),
        status,
        usage_columns(&record.usage),
        equivalent,
        usd_suffix(usd)
    );
    if !record.complete {
        let reason = record.error.as_deref().unwrap_or("unknown");
        let _ = write!(row, " INCOMPLETE: {reason}");
    }
    row
}

pub fn live_row(record: &CallRecord) -> Option<String> {
    (record.kind != CallKind::Other).then(|| format_row(record, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::provider::RequestFacts;
    use crate::wire::record::RequestBody;
    use chrono::{TimeZone, Utc};

    fn record(usage: UsageOutcome) -> CallRecord {
        let at = Utc.with_ymd_and_hms(2026, 10, 21, 16, 24, 29).unwrap();
        CallRecord {
            seq: 3,
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
            input_equivalent: Some(1234.4),
            complete: true,
            error: None,
            response_events: None,
        }
    }

    fn usage(writes: Vec<CacheWrite>) -> Usage {
        Usage {
            input_uncached: 100,
            cache_read: 600,
            cache_writes: writes,
            output: 20,
            reasoning: Some(5),
            other: vec![("x".into(), 1)],
        }
    }

    #[test]
    fn other_calls_have_no_row() {
        let mut r = record(UsageOutcome::Missing);
        r.kind = CallKind::Other;
        assert_eq!(live_row(&r), None);
    }

    #[test]
    fn full_row_with_tiers() {
        let writes = vec![
            CacheWrite {
                tier: CacheTier::Ttl { seconds: 300 },
                tokens: 200,
            },
            CacheWrite {
                tier: CacheTier::Ttl { seconds: 3600 },
                tokens: 100,
            },
            CacheWrite {
                tier: CacheTier::Unspecified,
                tokens: 0,
            },
        ];
        let row = live_row(&record(UsageOutcome::Reported(usage(writes)))).unwrap();
        assert_eq!(
            row,
            "#3 16:24:29 m 200 in=100 cr=600 w5m=200 w1h=100 w?=0 out=20 hit=60% eq=1234"
        );
    }

    #[test]
    fn incomplete_missing_usage_row() {
        let mut r = record(UsageOutcome::Missing);
        r.complete = false;
        r.error = Some("client disconnected".into());
        r.input_equivalent = None;
        r.status = None;
        assert_eq!(
            live_row(&r).unwrap(),
            "#3 16:24:29 m - usage=missing eq=- INCOMPLETE: client disconnected"
        );
    }

    #[test]
    fn zero_totals_and_no_writes() {
        let mut u = usage(vec![]);
        u.input_uncached = 0;
        u.cache_read = 0;
        let row = live_row(&record(UsageOutcome::Partial(u))).unwrap();
        assert!(row.contains("w=0 out=20 hit=-"), "{row}");
    }
}
