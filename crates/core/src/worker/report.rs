use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportStatus {
    Done,
    Blocked,
    Question,
}

impl ReportStatus {
    pub const ALL: [ReportStatus; 3] = [Self::Done, Self::Blocked, Self::Question];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Question => "question",
        }
    }

    pub fn parse(status: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.as_str() == status)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    NeedsUser,
    Informs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Worker(ReportStatus),
    Check,
    Warning,
    Watch,
    Merged,
    View,
    ThreadClosed,
    External,
}

impl ReportKind {
    pub const ALL: [ReportKind; 10] = [
        Self::Worker(ReportStatus::Done),
        Self::Worker(ReportStatus::Blocked),
        Self::Worker(ReportStatus::Question),
        Self::Check,
        Self::Warning,
        Self::Watch,
        Self::Merged,
        Self::View,
        Self::ThreadClosed,
        Self::External,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Worker(status) => status.as_str(),
            Self::Check => "check",
            Self::Warning => "warning",
            Self::Watch => "watch",
            Self::Merged => "merged",
            Self::View => "view",
            Self::ThreadClosed => "thread-closed",
            Self::External => "report",
        }
    }

    pub fn parse(stored: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.as_str() == stored)
    }

    pub fn worker_status(self) -> Option<ReportStatus> {
        match self {
            Self::Worker(status) => Some(status),
            Self::Check
            | Self::Warning
            | Self::Watch
            | Self::Merged
            | Self::View
            | Self::ThreadClosed
            | Self::External => None,
        }
    }

    pub fn attention(self) -> Attention {
        match self {
            Self::Worker(ReportStatus::Blocked | ReportStatus::Question) => Attention::NeedsUser,
            Self::Worker(ReportStatus::Done)
            | Self::Check
            | Self::Warning
            | Self::Watch
            | Self::Merged
            | Self::View
            | Self::ThreadClosed
            | Self::External => Attention::Informs,
        }
    }
}

impl fmt::Display for ReportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for ReportKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ReportKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ReportKindVisitor;

        impl Visitor<'_> for ReportKindVisitor {
            type Value = ReportKind;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a known report status string")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                ReportKind::parse(value)
                    .ok_or_else(|| E::custom(format!("unknown report status '{value}'")))
            }
        }

        deserializer.deserialize_str(ReportKindVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::Report;
    use serde_json::{json, Value};

    #[test]
    fn report_kind_round_trips_every_stored_string() {
        let expected = [
            ("done", ReportKind::Worker(ReportStatus::Done)),
            ("blocked", ReportKind::Worker(ReportStatus::Blocked)),
            ("question", ReportKind::Worker(ReportStatus::Question)),
            ("check", ReportKind::Check),
            ("warning", ReportKind::Warning),
            ("watch", ReportKind::Watch),
            ("merged", ReportKind::Merged),
            ("view", ReportKind::View),
            ("thread-closed", ReportKind::ThreadClosed),
            ("report", ReportKind::External),
        ];
        assert_eq!(expected.len(), ReportKind::ALL.len());
        for (stored, kind) in expected {
            assert_eq!(kind.as_str(), stored);
            assert_eq!(ReportKind::parse(stored), Some(kind));
            assert_eq!(kind.to_string(), stored);
            let encoded = serde_json::to_string(&kind).unwrap();
            assert_eq!(encoded, format!("\"{stored}\""));
            assert_eq!(serde_json::from_str::<ReportKind>(&encoded).unwrap(), kind);
        }
    }

    #[test]
    fn unknown_status_parses_to_none() {
        for raw in ["", "unknown", "failed", "waiting", "DONE", "Blocked"] {
            assert_eq!(ReportKind::parse(raw), None, "raw={raw:?}");
            assert!(serde_json::from_value::<ReportKind>(json!(raw)).is_err());
        }
    }

    #[test]
    fn ralph_blocked_report_serializes_as_blocked() {
        let report = Report::ralph_blocked("w1", "2026-11-25T00:00:00Z", "stuck".to_string());
        let encoded: Value = serde_json::to_value(&report).unwrap();
        assert_eq!(
            encoded,
            json!({
                "worker": "w1",
                "at": "2026-11-25T00:00:00Z",
                "status": "blocked",
                "text": "stuck",
            })
        );
    }

    #[test]
    fn worker_status_and_attention_cover_all_kinds() {
        for kind in ReportKind::ALL {
            match kind {
                ReportKind::Worker(status) => {
                    assert_eq!(kind.worker_status(), Some(status));
                    let expected = match status {
                        ReportStatus::Blocked | ReportStatus::Question => Attention::NeedsUser,
                        ReportStatus::Done => Attention::Informs,
                    };
                    assert_eq!(kind.attention(), expected);
                }
                ReportKind::Check
                | ReportKind::Warning
                | ReportKind::Watch
                | ReportKind::Merged
                | ReportKind::View
                | ReportKind::ThreadClosed
                | ReportKind::External => {
                    assert_eq!(kind.worker_status(), None);
                    assert_eq!(kind.attention(), Attention::Informs);
                }
            }
        }
    }

    #[test]
    fn constructors_set_expected_wire_status() {
        let cases = [
            (
                Report::from_worker("w1", "t", ReportStatus::Done, "ok".into()),
                "done",
            ),
            (
                Report::from_worker("w1", "t", ReportStatus::Blocked, "no".into()),
                "blocked",
            ),
            (
                Report::from_worker("w1", "t", ReportStatus::Question, "q".into()),
                "question",
            ),
            (Report::ralph_blocked("w1", "t", "p".into()), "blocked"),
            (Report::check("w1", "t", "c".into()), "check"),
            (Report::warning("w1", "t", "w".into()), "warning"),
            (Report::watch("w1", "t", "x".into()), "watch"),
            (Report::merged("w1", "t", "m".into()), "merged"),
            (Report::view("w1", "t", "v".into()), "view"),
            (
                Report::thread_closed("w1", "t", "tc".into()),
                "thread-closed",
            ),
            (Report::external("w1", "t", "e".into()), "report"),
        ];
        for (report, expected) in cases {
            assert_eq!(serde_json::to_value(&report).unwrap()["status"], expected);
        }
    }
}
