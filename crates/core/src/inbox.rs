use crate::worker::Report;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReportId(u64);

impl fmt::Display for ReportId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
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
}
