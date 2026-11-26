use crate::chat_thread_auto::is_close_report;
use crate::worker::{chat_message, Report};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::num::NonZeroUsize;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position(NonZeroUsize);

impl Position {
    pub const FIRST: Position = Position(NonZeroUsize::MIN);

    pub fn at_index(index: usize) -> Option<Self> {
        NonZeroUsize::new(index.checked_add(1)?).map(Self)
    }

    fn index(self) -> usize {
        self.0.get() - 1
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Position {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub source: String,
    pub report: Report,
}

#[derive(Debug)]
pub enum DecodeError {
    NotJson(serde_json::Error),
    MissingEntries,
    EntriesNotArray,
    BadEntry(serde_json::Error),
    BadMessage(serde_json::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson(error) => write!(f, "not valid JSON: {error}"),
            Self::MissingEntries => write!(f, "missing `entries` array"),
            Self::EntriesNotArray => write!(f, "`entries` is not an array"),
            Self::BadEntry(error) => write!(f, "malformed ledger entry: {error}"),
            Self::BadMessage(error) => write!(f, "malformed message: {error}"),
        }
    }
}

impl std::error::Error for DecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotJson(error) | Self::BadEntry(error) | Self::BadMessage(error) => Some(error),
            Self::MissingEntries | Self::EntriesNotArray => None,
        }
    }
}

#[derive(Debug)]
pub struct Skipped {
    pub index: Option<usize>,
    pub source: Option<String>,
    pub reason: DecodeError,
}

impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.index, self.source.as_deref()) {
            (Some(index), Some(source)) => {
                write!(f, "entry #{} ({source}): {}", index + 1, self.reason)
            }
            (Some(index), None) => write!(f, "entry #{}: {}", index + 1, self.reason),
            (None, Some(source)) => write!(f, "{source}: {}", self.reason),
            (None, None) => write!(f, "{}", self.reason),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Ledger {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct StoredEntry {
    source: String,
    #[serde(default)]
    read: bool,
    report: Report,
}

pub fn decode_message(bytes: &[u8]) -> Result<Report, DecodeError> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(DecodeError::NotJson)?;
    serde_json::from_value(value).map_err(DecodeError::BadMessage)
}

impl Ledger {
    pub fn decode(bytes: &[u8]) -> (Self, Vec<Skipped>) {
        let value: serde_json::Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                return (
                    Self::default(),
                    vec![Skipped {
                        index: None,
                        source: None,
                        reason: DecodeError::NotJson(error),
                    }],
                );
            }
        };
        let items = match value {
            serde_json::Value::Object(mut map) => match map.remove("entries") {
                Some(serde_json::Value::Array(items)) => items,
                Some(_) => {
                    return (
                        Self::default(),
                        vec![Skipped {
                            index: None,
                            source: None,
                            reason: DecodeError::EntriesNotArray,
                        }],
                    );
                }
                None => {
                    return (
                        Self::default(),
                        vec![Skipped {
                            index: None,
                            source: None,
                            reason: DecodeError::MissingEntries,
                        }],
                    );
                }
            },
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_)
            | serde_json::Value::Array(_) => {
                return (
                    Self::default(),
                    vec![Skipped {
                        index: None,
                        source: None,
                        reason: DecodeError::MissingEntries,
                    }],
                );
            }
        };
        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        for (index, raw) in items.into_iter().enumerate() {
            let source = raw
                .get("source")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            match serde_json::from_value::<StoredEntry>(raw) {
                Ok(entry) if entry.read => {}
                Ok(entry) => entries.push(Entry {
                    source: entry.source,
                    report: entry.report,
                }),
                Err(error) => skipped.push(Skipped {
                    index: Some(index),
                    source,
                    reason: DecodeError::BadEntry(error),
                }),
            }
        }
        (Self { entries }, skipped)
    }

    pub fn ingest(&mut self, arrivals: Vec<(String, Report)>) {
        for (source, report) in arrivals {
            let entry = Entry { source, report };
            if !self.entries.contains(&entry) {
                self.entries.push(entry);
            }
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, position: Position) -> Option<&Entry> {
        self.entries.get(position.index())
    }

    pub fn remove(&mut self, position: Position) -> Option<Entry> {
        (position.index() < self.entries.len()).then(|| self.entries.remove(position.index()))
    }

    pub fn reports(&self) -> Vec<Report> {
        self.entries.iter().map(|e| e.report.clone()).collect()
    }

    pub fn drop_worker(&mut self, worker: &str) -> Vec<Entry> {
        let (dropped, kept): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|e| e.report.worker == worker);
        self.entries = kept;
        dropped
    }

    pub fn take_closes(&mut self) -> Vec<Report> {
        let (closes, rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|e| is_close_report(&e.report));
        self.entries = rest;
        closes.into_iter().map(|e| e.report).collect()
    }

    pub fn take(&mut self, selection: Selection) -> Vec<Report> {
        let position = match selection {
            Selection::All => return self.entries.drain(..).map(|e| e.report).collect(),
            Selection::Next => Position::FIRST,
            Selection::At(position) => position,
        };
        self.remove(position)
            .map(|e| e.report)
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    All,
    Next,
    At(Position),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadTurn {
    pub logged: Vec<String>,
    pub message: String,
}

pub fn unread_turn(selection: Selection, reports: &[Report]) -> Result<UnreadTurn, String> {
    if reports.is_empty() {
        return Err(match selection {
            Selection::All | Selection::Next => "No unread reports.".to_string(),
            Selection::At(position) => {
                format!("No report {position}. /inbox list shows the queue.")
            }
        });
    }
    Ok(UnreadTurn {
        logged: reports
            .iter()
            .map(|report| chat_message(std::slice::from_ref(report)))
            .collect(),
        message: chat_message(reports),
    })
}

pub fn drop_log_line(entry: &Entry, now: DateTime<Utc>) -> String {
    serde_json::json!({
        "timestamp": now.to_rfc3339(),
        "worker": entry.report.worker,
        "status": entry.report.status,
        "text": entry.report.text,
    })
    .to_string()
}

pub fn age(at: &str, now: DateTime<Utc>) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(at) else {
        return "?".to_string();
    };
    let secs = (now - at.with_timezone(&Utc)).num_seconds().max(0);
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

pub fn list(entries: &[Entry], now: DateTime<Utc>) -> String {
    if entries.is_empty() {
        return "No reports.".to_string();
    }
    let rows: Vec<[String; 4]> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            [
                format!("#{}", i + 1),
                e.report.worker.clone(),
                e.report.status.clone(),
                age(&e.report.at, now),
            ]
        })
        .collect();
    let widths: [usize; 3] =
        std::array::from_fn(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0));
    rows.iter()
        .map(|r| {
            format!(
                "{:<w0$}  {:<w1$}  {:<w2$}  {}",
                r[0],
                r[1],
                r[2],
                r[3],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arrival(source: &str) -> (String, Report) {
        (
            source.to_string(),
            Report {
                worker: "A".to_string(),
                at: "t".to_string(),
                status: "done".to_string(),
                text: source.to_string(),
            },
        )
    }

    fn pos(n: usize) -> Position {
        n.to_string().parse().unwrap()
    }

    fn sources(ledger: &Ledger) -> Vec<&str> {
        ledger.entries.iter().map(|e| e.source.as_str()).collect()
    }

    fn entry(worker: &str, at: &str) -> Entry {
        Entry {
            source: worker.to_string(),
            report: Report {
                worker: worker.to_string(),
                at: at.to_string(),
                status: "done".to_string(),
                text: String::new(),
            },
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-01-10T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn drop_worker_removes_only_that_workers_entries_in_order() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![
            ("1".to_string(), entry("A", "t1").report),
            ("2".to_string(), entry("B", "t2").report),
            ("3".to_string(), entry("A", "t3").report),
        ]);
        let dropped = ledger.drop_worker("A");
        let times: Vec<&str> = dropped.iter().map(|e| e.report.at.as_str()).collect();
        assert_eq!(times, ["t1", "t3"]);
        assert_eq!(sources(&ledger), ["2"]);
        assert!(ledger.drop_worker("A").is_empty());
    }

    #[test]
    fn ingest_skips_known_sources() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        ledger.ingest(vec![arrival("b"), arrival("c")]);
        assert_eq!(sources(&ledger), ["a", "b", "c"]);
    }

    #[test]
    fn ingest_keeps_a_different_report_under_a_known_source() {
        let mut ledger = Ledger::default();
        let (source, first) = arrival("a");
        let second = Report {
            text: "second".to_string(),
            ..first.clone()
        };
        ledger.ingest(vec![(source.clone(), first.clone())]);
        ledger.ingest(vec![(source, second.clone())]);
        assert_eq!(ledger.reports(), [first, second]);
    }

    #[test]
    fn at_index_is_one_based_and_total() {
        assert_eq!(Position::at_index(0), Some(Position::FIRST));
        assert_eq!(Position::at_index(4).unwrap().to_string(), "5");
        assert_eq!(Position::at_index(usize::MAX), None);
    }

    #[test]
    fn position_parses_positive_numbers_only() {
        assert_eq!("12".parse::<Position>().unwrap().to_string(), "12");
        for bad in ["0", "-1", "x", ""] {
            assert!(bad.parse::<Position>().is_err(), "{bad}");
        }
    }

    #[test]
    fn get_finds_by_position_and_removes_nothing() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        assert_eq!(ledger.get(pos(2)).unwrap().source, "b");
        assert!(ledger.get(pos(3)).is_none());
        assert_eq!(ledger.reports().len(), 2);
        assert_eq!(sources(&ledger), ["a", "b"]);
    }

    #[test]
    fn remove_shifts_later_positions() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b"), arrival("c")]);
        assert_eq!(ledger.remove(pos(2)).unwrap().source, "b");
        assert!(ledger.remove(pos(3)).is_none());
        assert_eq!(ledger.get(pos(2)).unwrap().source, "c");
    }

    #[test]
    fn take_none_empties_the_queue_and_new_arrivals_start_at_one() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        let texts: Vec<_> = ledger
            .take(Selection::All)
            .into_iter()
            .map(|r| r.text)
            .collect();
        assert_eq!(texts, ["a", "b"]);
        assert!(ledger.entries().is_empty());
        ledger.ingest(vec![arrival("c")]);
        assert_eq!(ledger.get(pos(1)).unwrap().source, "c");
    }

    #[test]
    fn take_some_returns_that_report_and_shifts_later_positions() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b"), arrival("c")]);
        let texts: Vec<_> = ledger
            .take(Selection::At(pos(2)))
            .into_iter()
            .map(|r| r.text)
            .collect();
        assert_eq!(texts, ["b"]);
        assert_eq!(sources(&ledger), ["a", "c"]);
        assert_eq!(ledger.get(pos(2)).unwrap().source, "c");
    }

    #[test]
    fn take_missing_position_returns_nothing_and_keeps_the_queue() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a")]);
        assert!(ledger.take(Selection::At(pos(2))).is_empty());
        assert_eq!(sources(&ledger), ["a"]);
    }

    #[test]
    fn take_next_takes_the_oldest_and_tolerates_an_empty_ledger() {
        let mut ledger = Ledger::default();
        assert!(ledger.take(Selection::Next).is_empty());
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        let texts: Vec<_> = ledger
            .take(Selection::Next)
            .into_iter()
            .map(|r| r.text)
            .collect();
        assert_eq!(texts, ["a"]);
        assert_eq!(sources(&ledger), ["b"]);
    }

    #[test]
    fn unread_turn_of_nothing_names_the_selection() {
        let err = |s| unread_turn(s, &[]).unwrap_err();
        assert_eq!(err(Selection::All), "No unread reports.");
        assert_eq!(err(Selection::Next), "No unread reports.");
        assert_eq!(
            err(Selection::At(pos(3))),
            "No report 3. /inbox list shows the queue."
        );
    }

    #[test]
    fn unread_turn_logs_each_report_and_sends_them_together() {
        let reports = vec![arrival("a").1, arrival("b").1];
        let turn = unread_turn(Selection::All, &reports).unwrap();
        assert_eq!(turn.logged.len(), 2);
        assert_eq!(turn.logged[1], chat_message(&reports[1..]));
        assert_eq!(turn.message, chat_message(&reports));
    }

    #[test]
    fn old_ledger_drops_read_entries_and_ignores_unknown_fields() {
        let report =
            |t: &str| serde_json::json!({"worker": "A", "at": "t", "status": "done", "text": t});
        let old = serde_json::json!({
            "next_id": 4,
            "entries": [
                {"id": 1, "source": "a", "read": true, "report": report("a")},
                {"id": 2, "source": "b", "read": false, "report": report("b")},
                {"id": 3, "source": "c", "report": report("c")},
            ],
        });
        let (ledger, skipped) = Ledger::decode(&serde_json::to_vec(&old).unwrap());
        assert!(skipped.is_empty());
        assert_eq!(sources(&ledger), ["b", "c"]);
    }

    #[test]
    fn serde_round_trip_uses_the_new_shape() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        let value = serde_json::to_value(&ledger).unwrap();
        let sorted_keys = |value: &serde_json::Value| {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(sorted_keys(&value), ["entries"]);
        assert_eq!(sorted_keys(&value["entries"][0]), ["report", "source"]);
        let (decoded, skipped) = Ledger::decode(&serde_json::to_vec(&value).unwrap());
        assert!(skipped.is_empty());
        assert_eq!(decoded, ledger);
    }

    #[test]
    fn drop_log_line_has_no_id() {
        let line = drop_log_line(&entry("A", "t"), "2026-01-02T03:04:05Z".parse().unwrap());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            serde_json::json!({
                "timestamp": "2026-01-02T03:04:05+00:00",
                "worker": "A",
                "status": "done",
                "text": "",
            })
        );
    }

    #[test]
    fn list_of_nothing_says_so() {
        assert_eq!(list(&[], now()), "No reports.");
    }

    #[test]
    fn list_numbers_by_position_and_pads_columns() {
        let mut entries: Vec<_> = (0..9).map(|_| entry("A", "2026-01-09T23:59:30Z")).collect();
        entries.push(entry("GUZ-1", "2026-01-09T23:00:00Z"));
        let out = list(&entries, now());
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines[0], "#1   A      done  30s");
        assert_eq!(lines[9], "#10  GUZ-1  done  1h");
    }

    #[test]
    fn age_picks_unit_and_clamps() {
        let at = |s| age(s, now());
        assert_eq!(at("2026-01-09T23:59:59Z"), "1s");
        assert_eq!(at("2026-01-09T23:58:00Z"), "2m");
        assert_eq!(at("2026-01-09T20:00:00Z"), "4h");
        assert_eq!(at("2026-01-07T00:00:00Z"), "3d");
        assert_eq!(at("2026-01-10T01:00:00Z"), "0s");
        assert_eq!(at("t"), "?");
    }

    #[test]
    fn take_closes_removes_only_thread_closed_reports() {
        use crate::chat_thread::{AutoMode, ThreadName};
        use crate::chat_thread_auto::{split_close_reports, thread_closed_report};
        let closed =
            thread_closed_report(&ThreadName::parse("side").unwrap(), AutoMode::Merge, "t");
        let mut ledger = Ledger::default();
        ledger.ingest(vec![
            arrival("a"),
            ("c".to_string(), closed.clone()),
            arrival("b"),
        ]);
        assert_eq!(ledger.take_closes(), vec![closed.clone()]);
        assert_eq!(sources(&ledger), ["a", "b"]);

        let mut only = Ledger::default();
        only.ingest(vec![("c".to_string(), closed)]);
        let (_, rest) = split_close_reports(only.take(Selection::All));
        assert!(unread_turn(Selection::All, &rest).is_err());
    }

    #[test]
    fn ledger_with_one_malformed_entry_keeps_the_good_entries() {
        let report =
            |t: &str| serde_json::json!({"worker": "A", "at": "t", "status": "done", "text": t});
        let raw = serde_json::json!({
            "entries": [
                {"source": "a", "report": report("a")},
                {"source": "bad", "report": {"worker": "A"}},
                {"source": "b", "report": report("b")},
            ],
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let (ledger, skipped) = Ledger::decode(&bytes);
        assert_eq!(sources(&ledger), ["a", "b"]);
        assert_eq!(skipped.len(), 1);
    }

    #[test]
    fn decode_reports_each_skipped_entry() {
        let report =
            |t: &str| serde_json::json!({"worker": "A", "at": "t", "status": "done", "text": t});
        let raw = serde_json::json!({
            "entries": [
                {"source": "bad1", "report": {"worker": "A"}},
                {"source": "good", "report": report("ok")},
                42,
            ],
        });
        let bytes = serde_json::to_vec(&raw).unwrap();
        let (ledger, skipped) = Ledger::decode(&bytes);
        assert_eq!(sources(&ledger), ["good"]);
        assert_eq!(skipped.len(), 2);
        assert_eq!(skipped[0].index, Some(0));
        assert_eq!(skipped[0].source.as_deref(), Some("bad1"));
        assert!(match &skipped[0].reason {
            DecodeError::BadEntry(_) => true,
            DecodeError::NotJson(_)
            | DecodeError::MissingEntries
            | DecodeError::EntriesNotArray
            | DecodeError::BadMessage(_) => false,
        });
        assert!(skipped[0].to_string().contains("bad1"));
        assert_eq!(skipped[1].index, Some(2));
        assert_eq!(skipped[1].source, None);
        assert!(match &skipped[1].reason {
            DecodeError::BadEntry(_) => true,
            DecodeError::NotJson(_)
            | DecodeError::MissingEntries
            | DecodeError::EntriesNotArray
            | DecodeError::BadMessage(_) => false,
        });

        let (empty, bad_file) = Ledger::decode(b"not json");
        assert!(empty.entries().is_empty());
        assert_eq!(bad_file.len(), 1);
        assert_eq!(bad_file[0].index, None);
        assert_eq!(bad_file[0].source, None);
        assert!(match &bad_file[0].reason {
            DecodeError::NotJson(_) => true,
            DecodeError::MissingEntries
            | DecodeError::EntriesNotArray
            | DecodeError::BadEntry(_)
            | DecodeError::BadMessage(_) => false,
        });

        let (missing_entries, bad_shape) = Ledger::decode(b"{}");
        assert!(missing_entries.entries().is_empty());
        assert_eq!(bad_shape.len(), 1);
        assert_eq!(bad_shape[0].index, None);
        assert!(match &bad_shape[0].reason {
            DecodeError::MissingEntries => true,
            DecodeError::NotJson(_)
            | DecodeError::EntriesNotArray
            | DecodeError::BadEntry(_)
            | DecodeError::BadMessage(_) => false,
        });

        let (wrong_entries, not_array) = Ledger::decode(br#"{"entries": 42}"#);
        assert!(wrong_entries.entries().is_empty());
        assert_eq!(not_array.len(), 1);
        assert_eq!(not_array[0].index, None);
        assert!(match &not_array[0].reason {
            DecodeError::EntriesNotArray => true,
            DecodeError::NotJson(_)
            | DecodeError::MissingEntries
            | DecodeError::BadEntry(_)
            | DecodeError::BadMessage(_) => false,
        });
    }

    #[test]
    fn decode_message_decodes_valid_report_and_rejects_malformed() {
        let report = arrival("a").1;
        let bytes = serde_json::to_vec(&report).unwrap();
        assert_eq!(decode_message(&bytes).unwrap(), report);
        assert!(match decode_message(b"not json").unwrap_err() {
            DecodeError::NotJson(_) => true,
            DecodeError::MissingEntries
            | DecodeError::EntriesNotArray
            | DecodeError::BadEntry(_)
            | DecodeError::BadMessage(_) => false,
        });
        assert!(match decode_message(br#"{"worker":"A"}"#).unwrap_err() {
            DecodeError::BadMessage(_) => true,
            DecodeError::NotJson(_)
            | DecodeError::MissingEntries
            | DecodeError::EntriesNotArray
            | DecodeError::BadEntry(_) => false,
        });
    }
}
