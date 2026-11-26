use super::ReportLine;
use crate::worker::{pending_requests, ActivityState};
use crate::worker_costs::window::instant;
use chrono::SecondsFormat;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WebTone {
    Ok,
    Bad,
    Warn,
    Info,
    Accent,
    Neutral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFacts {
    pub id: String,
    pub harness: String,
    pub blocked: Option<String>,
    pub waiting: bool,
    pub unreported: bool,
    pub gone: bool,
    pub activity: Option<ActivityState>,
    pub ticket: Option<String>,
    pub last_report: Option<ReportLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerWebRow {
    pub id: String,
    pub harness: String,
    pub state: &'static str,
    pub tone: WebTone,
    pub pending_count: usize,
    pub ticket: Option<String>,
    pub last_report_status: Option<String>,
    pub last_report_at: Option<String>,
}

fn activity_tone(activity: ActivityState) -> WebTone {
    match activity {
        ActivityState::Running | ActivityState::Background => WebTone::Ok,
        ActivityState::Idle => WebTone::Neutral,
        ActivityState::Failed => WebTone::Bad,
    }
}

fn state_and_tone(facts: &WorkerFacts) -> (&'static str, WebTone) {
    if facts.gone {
        ("gone", WebTone::Neutral)
    } else if facts.blocked.is_some() {
        ("blocked", WebTone::Warn)
    } else if facts.waiting {
        ("waiting", WebTone::Info)
    } else if facts.unreported {
        ("unreported", WebTone::Accent)
    } else {
        facts.activity.map_or(("running", WebTone::Ok), |activity| {
            (activity.label(), activity_tone(activity))
        })
    }
}

/// A ticket key is shown only with the shape `^[A-Z][A-Z0-9]*-[0-9]+$`, with a
/// project of at most 10 characters and a number of at most 8 digits.
/// Any other text in the spawn record is dropped, not masked.
const MAX_PROJECT_LEN: usize = 10;
const MAX_NUMBER_LEN: usize = 8;

pub fn ticket_shape(key: &str) -> Option<&str> {
    let (project, number) = key.split_once('-')?;
    let mut letters = project.chars();
    let valid = project.len() <= MAX_PROJECT_LEN
        && number.len() <= MAX_NUMBER_LEN
        && letters.next().is_some_and(|c| c.is_ascii_uppercase())
        && letters.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit());
    valid.then_some(key)
}

/// A report time is returned only when it parses as RFC 3339, and then in the
/// canonical UTC form. The log holds the raw text, which may be anything.
fn canonical_instant(at: &str) -> Option<String> {
    instant(at).map(|at| at.to_rfc3339_opts(SecondsFormat::Secs, true))
}

pub fn worker_state_rows(facts: &[WorkerFacts]) -> Vec<WorkerWebRow> {
    let mut sorted: Vec<&WorkerFacts> = facts.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    sorted
        .into_iter()
        .map(|worker| {
            let (state, tone) = state_and_tone(worker);
            WorkerWebRow {
                id: worker.id.clone(),
                harness: worker.harness.clone(),
                state,
                tone,
                pending_count: worker
                    .blocked
                    .as_deref()
                    .map_or(0, |marker| pending_requests(marker).len()),
                ticket: worker
                    .ticket
                    .as_deref()
                    .and_then(ticket_shape)
                    .map(str::to_string),
                last_report_status: worker
                    .last_report
                    .as_ref()
                    .map(|r| r.status.as_str().to_string()),
                last_report_at: worker
                    .last_report
                    .as_ref()
                    .and_then(|r| canonical_instant(&r.at)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{ActivityState, ReportKind, ReportStatus};

    #[test]
    fn only_ticket_shaped_keys_pass() {
        for ok in ["GUZ-1", "GUZ-1748", "A-0", "AB2-30", "ABCDEFGHIJ-12345678"] {
            assert_eq!(ticket_shape(ok), Some(ok), "{ok}");
        }
        for bad in [
            "sk-live-0123456789abcdef",
            "guz-1",
            "GUZ-",
            "-1",
            "GUZ-1-2",
            "GUZ-12a",
            "2GUZ-1",
            "GUZ-1\n",
            "",
            "A-4111111111111111",
            "A-123456789",
            "ABCDEFGHIJK-1",
        ] {
            assert_eq!(ticket_shape(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_report_time_is_returned_only_as_a_canonical_instant() {
        let at = |text: &str| {
            let mut worker = facts("w");
            worker.last_report = Some(ReportLine {
                status: ReportKind::Worker(ReportStatus::Done),
                at: text.to_string(),
                text: "body".to_string(),
            });
            worker_state_rows(&[worker])[0].last_report_at.clone()
        };
        let canonical = Some("2026-03-01T09:00:00Z".to_string());
        assert_eq!(at("2026-03-01T09:00:00Z"), canonical);
        assert_eq!(at("2026-03-01T10:00:00+01:00"), canonical);
        assert_eq!(at("2026-03-01T09:00:00.123Z"), canonical);
        assert_eq!(at("sk-live-0123456789abcdef"), None);
        assert_eq!(at(""), None);
    }

    fn facts(id: &str) -> WorkerFacts {
        WorkerFacts {
            id: id.to_string(),
            harness: "claude".to_string(),
            blocked: None,
            waiting: false,
            unreported: false,
            gone: false,
            activity: None,
            ticket: None,
            last_report: None,
        }
    }

    fn state_of(facts: &WorkerFacts) -> (&'static str, WebTone) {
        let rows = worker_state_rows(std::slice::from_ref(facts));
        (rows[0].state, rows[0].tone)
    }

    #[test]
    fn a_worker_with_no_marks_is_running() {
        assert_eq!(state_of(&facts("a")), ("running", WebTone::Ok));
    }

    #[test]
    fn each_activity_label_has_its_tone() {
        let cases = [
            (ActivityState::Running, "running", WebTone::Ok),
            (ActivityState::Background, "background", WebTone::Ok),
            (ActivityState::Idle, "idle", WebTone::Neutral),
            (ActivityState::Failed, "failed", WebTone::Bad),
        ];
        for (activity, label, tone) in cases {
            let mut worker = facts("a");
            worker.activity = Some(activity);
            assert_eq!(state_of(&worker), (label, tone));
        }
    }

    #[test]
    fn the_marks_beat_the_activity_in_a_fixed_order() {
        let mut worker = facts("a");
        worker.activity = Some(ActivityState::Failed);
        worker.unreported = true;
        assert_eq!(state_of(&worker), ("unreported", WebTone::Accent));
        worker.waiting = true;
        assert_eq!(state_of(&worker), ("waiting", WebTone::Info));
        worker.blocked = Some("{}".to_string());
        assert_eq!(state_of(&worker), ("blocked", WebTone::Warn));
        worker.gone = true;
        assert_eq!(state_of(&worker), ("gone", WebTone::Neutral));
    }

    #[test]
    fn pending_counts_the_blocked_marker_requests() {
        let mut worker = facts("a");
        assert_eq!(worker_state_rows(&[worker.clone()])[0].pending_count, 0);
        worker.blocked = Some(r#"{"request_id":"r1"}"#.to_string());
        assert_eq!(worker_state_rows(&[worker.clone()])[0].pending_count, 1);
        worker.blocked = Some(r#"{"request_id":"r1","pending":["r1","r2","r3"]}"#.to_string());
        assert_eq!(worker_state_rows(&[worker.clone()])[0].pending_count, 3);
        worker.blocked = Some("not json".to_string());
        assert_eq!(worker_state_rows(&[worker])[0].pending_count, 0);
    }

    #[test]
    fn rows_are_sorted_by_id_and_carry_ticket_and_last_report() {
        let mut second = facts("b");
        second.ticket = Some("GUZ-9".to_string());
        second.last_report = Some(ReportLine {
            status: ReportKind::Worker(ReportStatus::Done),
            at: "2026-01-01T00:00:00Z".to_string(),
            text: "secret /Users/alice".to_string(),
        });
        let rows = worker_state_rows(&[second, facts("a")]);
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        assert_eq!(
            serde_json::to_value(&rows[1]).unwrap(),
            serde_json::json!({
                "id": "b", "harness": "claude", "state": "running", "tone": "ok",
                "pending_count": 0, "ticket": "GUZ-9",
                "last_report_status": "done", "last_report_at": "2026-01-01T00:00:00Z",
            })
        );
        assert_eq!(
            serde_json::to_value(&rows[0]).unwrap()["ticket"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn the_serialized_row_never_carries_report_text_or_marker_text() {
        let mut worker = facts("a");
        worker.blocked = Some(r#"{"request_id":"r1","summary":"rm /Users/alice"}"#.to_string());
        worker.last_report = Some(ReportLine {
            status: ReportKind::Worker(ReportStatus::Done),
            at: "t".to_string(),
            text: "report-text".to_string(),
        });
        let json = serde_json::to_string(&worker_state_rows(&[worker])).unwrap();
        for leak in ["report-text", "/Users", "rm "] {
            assert!(!json.contains(leak), "{leak}");
        }
    }

    #[test]
    fn nothing_gives_no_rows() {
        assert!(worker_state_rows(&[]).is_empty());
    }
}
