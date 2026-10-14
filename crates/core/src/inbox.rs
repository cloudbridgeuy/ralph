use crate::worker::Report;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReportId(u64);

impl fmt::Display for ReportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ReportId {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: ReportId,
    pub source: String,
    pub read: bool,
    pub report: Report,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    next_id: u64,
    entries: Vec<Entry>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            next_id: 1,
            entries: Vec::new(),
        }
    }
}

impl Ledger {
    pub fn ingest(&mut self, arrivals: Vec<(String, Report)>) {
        for (source, report) in arrivals {
            if self.entries.iter().any(|e| e.source == source) {
                continue;
            }
            self.entries.push(Entry {
                id: ReportId(self.next_id),
                source,
                read: false,
                report,
            });
            self.next_id += 1;
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: ReportId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn remove(&mut self, id: ReportId) -> Option<Entry> {
        let at = self.entries.iter().position(|e| e.id == id)?;
        Some(self.entries.remove(at))
    }

    pub fn unread(&self) -> Vec<Report> {
        self.entries
            .iter()
            .filter(|e| !e.read)
            .map(|e| e.report.clone())
            .collect()
    }

    pub fn read_unread(&mut self) -> Vec<Report> {
        self.entries
            .iter_mut()
            .filter(|e| !e.read)
            .map(|e| {
                e.read = true;
                e.report.clone()
            })
            .collect()
    }
}

pub fn drop_log_line(entry: &Entry, now: DateTime<Utc>) -> String {
    serde_json::json!({
        "timestamp": now.to_rfc3339(),
        "id": entry.id,
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
    let rows: Vec<[String; 5]> = entries
        .iter()
        .map(|e| {
            [
                format!("#{}", e.id),
                e.report.worker.clone(),
                e.report.status.clone(),
                age(&e.report.at, now),
                if e.read { "read" } else { "unread" }.to_string(),
            ]
        })
        .collect();
    let widths: [usize; 4] =
        std::array::from_fn(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0));
    rows.iter()
        .map(|r| {
            format!(
                "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}",
                r[0],
                r[1],
                r[2],
                r[3],
                r[4],
                w0 = widths[0],
                w1 = widths[1],
                w2 = widths[2],
                w3 = widths[3],
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

    fn ids(ledger: &Ledger) -> Vec<String> {
        ledger.entries.iter().map(|e| e.id.to_string()).collect()
    }

    #[test]
    fn ids_follow_arrival_order_from_one() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        assert_eq!(ids(&ledger), ["1", "2"]);
    }

    #[test]
    fn duplicate_source_is_skipped() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a")]);
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        assert_eq!(ids(&ledger), ["1", "2"]);
    }

    #[test]
    fn reading_keeps_ids_and_later_arrivals_continue() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        assert_eq!(ledger.read_unread().len(), 2);
        assert!(ledger.unread().is_empty());
        ledger.ingest(vec![arrival("c")]);
        assert_eq!(ids(&ledger), ["1", "2", "3"]);
        assert_eq!(ledger.unread().len(), 1);
        assert!(ledger.entries[..2].iter().all(|e| e.read));
    }

    #[test]
    fn remove_takes_only_that_entry_and_never_reuses_its_id() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b"), arrival("c")]);
        let id = ledger.entries[2].id;
        assert_eq!(ledger.remove(id).map(|e| e.report.text), Some("c".into()));
        assert_eq!(ledger.remove(id), None);
        ledger.ingest(vec![arrival("d")]);
        assert_eq!(ids(&ledger), ["1", "2", "4"]);
    }

    #[test]
    fn drop_log_line_is_one_json_object_with_the_report() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a")]);
        let now = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .unwrap()
            .with_timezone(&Utc);
        let line = drop_log_line(&ledger.entries[0], now);
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "timestamp": "2026-01-02T03:04:05+00:00",
                "id": 1,
                "worker": "A",
                "status": "done",
                "text": "a",
            })
        );
    }

    #[test]
    fn removed_id_is_not_reused() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b"), arrival("c")]);
        ledger.entries.retain(|e| e.id.to_string() != "2");
        assert_eq!(ids(&ledger), ["1", "3"]);
        ledger.entries.retain(|e| e.id.to_string() != "3");
        ledger.ingest(vec![arrival("d")]);
        assert_eq!(ids(&ledger), ["1", "4"]);
    }

    #[test]
    fn serde_round_trip() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        ledger.read_unread();
        ledger.ingest(vec![arrival("c")]);
        let json = serde_json::to_string(&ledger).unwrap();
        assert_eq!(serde_json::from_str::<Ledger>(&json).unwrap(), ledger);
    }

    fn entry(id: &str, worker: &str, at: &str, read: bool) -> Entry {
        Entry {
            id: id.parse().unwrap(),
            source: id.to_string(),
            read,
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
    fn report_id_parses_numbers_only() {
        assert_eq!("12".parse::<ReportId>().unwrap().to_string(), "12");
        assert!("x".parse::<ReportId>().is_err());
        assert!("-1".parse::<ReportId>().is_err());
    }

    #[test]
    fn get_finds_by_id() {
        let mut ledger = Ledger::default();
        ledger.ingest(vec![arrival("a"), arrival("b")]);
        assert_eq!(ledger.get("2".parse().unwrap()).unwrap().source, "b");
        assert!(ledger.get("3".parse().unwrap()).is_none());
    }

    #[test]
    fn list_of_nothing_says_so() {
        assert_eq!(list(&[], now()), "No reports.");
    }

    #[test]
    fn list_pads_columns_and_marks_read_state() {
        let entries = [
            entry("1", "A", "2026-01-09T23:59:30Z", true),
            entry("10", "GUZ-1", "2026-01-09T23:00:00Z", false),
        ];
        assert_eq!(
            list(&entries, now()),
            "#1   A      done  30s  read\n#10  GUZ-1  done  1h   unread"
        );
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
