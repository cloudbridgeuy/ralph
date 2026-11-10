use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TIMING_VERSION: u32 = 1;
const ANALYTICS_DIR: &str = "analytics";
const TIMING_FILE: &str = "timing.jsonl";

pub fn timing_path(store: &Path) -> PathBuf {
    store.join(ANALYTICS_DIR).join(TIMING_FILE)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Copy)]
pub struct TimingKey<'a> {
    pub store: &'a str,
    pub run: Option<&'a str>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Ok,
    Rejected,
    Failed,
    Timeout,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnTiming {
    memory_wait_ms: u64,
    outage_wait_ms: u64,
    outage_retries: u64,
    model_ms: Option<u64>,
}

impl TurnTiming {
    pub fn with_memory_wait(self, waited: Duration) -> TurnTiming {
        TurnTiming {
            memory_wait_ms: self.memory_wait_ms.saturating_add(millis(waited)),
            ..self
        }
    }

    pub fn with_outage_wait(self, waited: Duration) -> TurnTiming {
        TurnTiming {
            outage_wait_ms: self.outage_wait_ms.saturating_add(millis(waited)),
            outage_retries: self.outage_retries.saturating_add(1),
            ..self
        }
    }

    pub fn with_model_ms(self, model_ms: Option<u64>) -> TurnTiming {
        TurnTiming { model_ms, ..self }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NapStats {
    pending_at_start: u64,
    calls: u64,
}

impl NapStats {
    pub fn start(pending_at_start: u64) -> NapStats {
        NapStats {
            pending_at_start,
            calls: 0,
        }
    }

    pub fn with_call(self) -> NapStats {
        NapStats {
            calls: self.calls.saturating_add(1),
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NapTimes {
    pub total: Duration,
    pub lock_wait: Duration,
    pub lock_held: Duration,
}

#[derive(Debug, Clone)]
pub struct NapCallFacts {
    pub node: String,
    pub attempt: u64,
    pub took: Duration,
    pub outcome: CallOutcome,
    pub exit: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum TimingEvent {
    Turn {
        turn_ms: u64,
        memory_wait_ms: u64,
        outage_wait_ms: u64,
        outage_retries: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_ms: Option<u64>,
        ok: bool,
    },
    Nap {
        total_ms: u64,
        pending_at_start: u64,
        calls: u64,
        lock_held_ms: u64,
        nap_lock_wait_ms: u64,
    },
    NapCall {
        node: String,
        attempt: u64,
        duration_ms: u64,
        outcome: CallOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimingRow {
    v: u32,
    at: String,
    store: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run: Option<String>,
    #[serde(flatten)]
    event: TimingEvent,
}

fn row(key: &TimingKey, event: TimingEvent) -> TimingRow {
    TimingRow {
        v: TIMING_VERSION,
        at: key.at.to_rfc3339_opts(SecondsFormat::Secs, true),
        store: key.store.to_string(),
        run: key.run.map(str::to_string),
        event,
    }
}

pub fn turn_row(key: &TimingKey, elapsed: Duration, timing: &TurnTiming, ok: bool) -> TimingRow {
    row(
        key,
        TimingEvent::Turn {
            turn_ms: millis(elapsed),
            memory_wait_ms: timing.memory_wait_ms,
            outage_wait_ms: timing.outage_wait_ms,
            outage_retries: timing.outage_retries,
            model_ms: timing.model_ms,
            ok,
        },
    )
}

pub fn nap_row(key: &TimingKey, stats: NapStats, times: NapTimes) -> TimingRow {
    row(
        key,
        TimingEvent::Nap {
            total_ms: millis(times.total),
            pending_at_start: stats.pending_at_start,
            calls: stats.calls,
            lock_held_ms: millis(times.lock_held),
            nap_lock_wait_ms: millis(times.lock_wait),
        },
    )
}

pub fn nap_call_row(key: &TimingKey, call: &NapCallFacts) -> TimingRow {
    row(
        key,
        TimingEvent::NapCall {
            node: call.node.clone(),
            attempt: call.attempt,
            duration_ms: millis(call.took),
            outcome: call.outcome,
            exit: call.exit,
        },
    )
}

pub fn encode_timing(row: &TimingRow) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(row)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use serde_json::{json, Value};
    use std::time::Duration;

    fn key() -> TimingKey<'static> {
        TimingKey {
            store: "/s",
            run: Some("r1"),
            at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        }
    }

    fn value(row: &TimingRow) -> Value {
        let line = encode_timing(row).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn the_log_sits_in_the_analytics_dir() {
        assert_eq!(
            timing_path(std::path::Path::new("/s")),
            std::path::Path::new("/s/analytics/timing.jsonl")
        );
    }

    #[test]
    fn a_turn_row_round_trips() {
        let timing = TurnTiming::default()
            .with_memory_wait(Duration::from_millis(30))
            .with_outage_wait(Duration::from_millis(200))
            .with_model_ms(Some(900));
        let row = turn_row(&key(), Duration::from_millis(1500), &timing, true);
        assert_eq!(
            value(&row),
            json!({
                "v": 1,
                "at": "2023-11-14T22:13:20Z",
                "store": "/s",
                "run": "r1",
                "event": "turn",
                "turn_ms": 1500,
                "memory_wait_ms": 30,
                "outage_wait_ms": 200,
                "outage_retries": 1,
                "model_ms": 900,
                "ok": true,
            })
        );
        let back: TimingRow = serde_json::from_str(&encode_timing(&row).unwrap()).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn readers_ignore_unknown_fields() {
        let line = r#"{"v":1,"at":"a","store":"/s","event":"nap_call","node":"0","attempt":1,"duration_ms":5,"outcome":"ok","future":{"x":1}}"#;
        let row: TimingRow = serde_json::from_str(line).unwrap();
        assert_eq!(value(&row)["outcome"], "ok");
    }

    #[test]
    fn none_fields_are_skipped() {
        let call = NapCallFacts {
            node: "0".into(),
            attempt: 1,
            took: Duration::from_millis(7),
            outcome: CallOutcome::Failed,
            exit: None,
        };
        let mut anon = key();
        anon.run = None;
        let v = value(&nap_call_row(&anon, &call));
        assert_eq!(v["outcome"], "failed");
        assert!(v.get("exit").is_none() && v.get("run").is_none());
        let timing = TurnTiming::default();
        let turn = value(&turn_row(&key(), Duration::ZERO, &timing, false));
        assert!(turn.get("model_ms").is_none());
        assert_eq!(turn["ok"], false);
    }

    #[test]
    fn a_timeout_carries_its_exit_code() {
        let call = NapCallFacts {
            node: "0-3".into(),
            attempt: 2,
            took: Duration::from_millis(9),
            outcome: CallOutcome::Timeout,
            exit: Some(-1),
        };
        let v = value(&nap_call_row(&key(), &call));
        assert_eq!(
            (v["outcome"].as_str(), v["exit"].as_i64()),
            (Some("timeout"), Some(-1))
        );
    }

    #[test]
    fn outage_waits_accumulate_by_count_and_sum() {
        let timing = [100u64, 250, 40]
            .into_iter()
            .map(Duration::from_millis)
            .fold(TurnTiming::default(), TurnTiming::with_outage_wait);
        let v = value(&turn_row(&key(), Duration::ZERO, &timing, true));
        assert_eq!(
            (v["outage_retries"].as_u64(), v["outage_wait_ms"].as_u64()),
            (Some(3), Some(390))
        );
    }

    #[test]
    fn memory_waits_accumulate() {
        let timing = TurnTiming::default()
            .with_memory_wait(Duration::from_millis(5))
            .with_memory_wait(Duration::from_millis(6));
        let v = value(&turn_row(&key(), Duration::ZERO, &timing, true));
        assert_eq!(v["memory_wait_ms"], 11);
    }

    #[test]
    fn nap_stats_count_calls_and_a_nap_row_reports_them() {
        let stats = NapStats::start(4).with_call().with_call();
        let row = nap_row(
            &key(),
            stats,
            NapTimes {
                total: Duration::from_millis(900),
                lock_wait: Duration::from_millis(100),
                lock_held: Duration::from_millis(800),
            },
        );
        let v = value(&row);
        assert_eq!(v["event"], "nap");
        assert_eq!(
            (v["pending_at_start"].as_u64(), v["calls"].as_u64()),
            (Some(4), Some(2))
        );
        assert_eq!(
            (
                v["total_ms"].as_u64(),
                v["nap_lock_wait_ms"].as_u64(),
                v["lock_held_ms"].as_u64()
            ),
            (Some(900), Some(100), Some(800))
        );
    }
}
