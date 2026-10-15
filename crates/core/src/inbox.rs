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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredLedger")]
pub struct Ledger {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct StoredLedger {
    entries: Vec<StoredEntry>,
}

#[derive(Deserialize)]
struct StoredEntry {
    source: String,
    #[serde(default)]
    read: bool,
    report: Report,
}

impl From<StoredLedger> for Ledger {
    fn from(stored: StoredLedger) -> Self {
        Self {
            entries: stored
                .entries
                .into_iter()
                .filter(|e| !e.read)
                .map(|e| Entry {
                    source: e.source,
                    report: e.report,
                })
                .collect(),
        }
    }
}

impl Ledger {
    pub fn ingest(&mut self, arrivals: Vec<(String, Report)>) {
        for (source, report) in arrivals {
            if self.entries.iter().any(|e| e.source == source) {
                continue;
            }
            self.entries.push(Entry { source, report });
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

fn age(at: &str, now: DateTime<Utc>) -> String {
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
    fn ingest_skips_known_sources() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        ledger.ingest(vec![arrival("b"), arrival("c")]);
        assert_eq!(sources(&ledger), ["a", "b", "c"]);
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
        let ledger: Ledger = serde_json::from_value(old).unwrap();
        assert_eq!(sources(&ledger), ["b", "c"]);
    }

    #[test]
    fn serde_round_trip_uses_the_new_shape() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        let value = serde_json::to_value(&ledger).unwrap();
        assert_eq!(
            value.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["entries"]
        );
        assert_eq!(
            value["entries"][0]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["report", "source"]
        );
        assert_eq!(serde_json::from_value::<Ledger>(value).unwrap(), ledger);
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
}
