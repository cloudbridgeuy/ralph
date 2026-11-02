use crate::worker::{ActivityMark, ActivityState};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const AUTO_CONTINUED_DIR: &str = "auto-continued";
pub const AUTO_CONTINUE_LOG: &str = "auto-continue.jsonl";
pub const REASON_API_ERROR: &str = "api error";
pub const CONTINUE_TEXT: &str = "Your last turn ended on an API error. Continue: start with git status, then follow the same rules as the last go-ahead.";

pub fn auto_continued_path(store: &Path, id: &str) -> PathBuf {
    store.join(AUTO_CONTINUED_DIR).join(id)
}

pub fn auto_continue_log_path(store: &Path) -> PathBuf {
    store.join(AUTO_CONTINUE_LOG)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoContinue {
    pub at: u64,
    pub reason: String,
    pub alerted: bool,
}

impl AutoContinue {
    pub fn new(at: u64, reason: &str) -> Self {
        Self {
            at,
            reason: reason.to_string(),
            alerted: false,
        }
    }

    pub fn alerted(self) -> Self {
        Self {
            alerted: true,
            ..self
        }
    }

    pub fn render(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text.trim()).ok()
    }
}

#[derive(Serialize)]
struct LogLine<'a> {
    at: u64,
    worker: &'a str,
    harness: &'a str,
    reason: &'a str,
}

pub fn log_line(at: u64, worker: &str, harness: &str, reason: &str) -> String {
    serde_json::to_string(&LogLine {
        at,
        worker,
        harness,
        reason,
    })
    .unwrap_or_default()
}

pub fn alert_text(id: &str, tmux_session: Option<&str>) -> String {
    let base = format!("Worker {id} is still idle with no report after one auto-continue.");
    match tmux_session {
        Some(session) => format!("{base} To look: tmux attach -t {session}"),
        None => base,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StallFacts {
    pub activity: Option<ActivityMark>,
    pub reported: bool,
    pub record: Option<AutoContinue>,
    pub sent_at: Option<u64>,
    pub now: u64,
    pub check_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinueStep {
    Log,
    RemoveRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordGate {
    pub send: bool,
    pub warn: bool,
}

pub fn gate_send(written: bool, warned_in_streak: bool) -> RecordGate {
    RecordGate {
        send: written,
        warn: !written && !warned_in_streak,
    }
}

pub fn after_send(sent: bool) -> &'static [ContinueStep] {
    if sent {
        &[ContinueStep::Log]
    } else {
        &[ContinueStep::RemoveRecord]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallAction {
    None,
    Continue,
    Alert,
}

const GRACE_CHECKS: u64 = 2;

pub fn grace_secs(check_secs: u64) -> u64 {
    check_secs.saturating_mul(GRACE_CHECKS)
}

fn went_unanswered(record: &AutoContinue, facts: &StallFacts) -> bool {
    facts.now >= record.at.saturating_add(grace_secs(facts.check_secs))
}

pub fn reported_since_go_ahead(sent_at: Option<u64>, report_at: Option<u64>) -> bool {
    match sent_at {
        None => true,
        Some(sent) => report_at.is_some_and(|report| report >= sent),
    }
}

pub fn report_secs(at: &str) -> Option<u64> {
    let parsed = DateTime::parse_from_rfc3339(at).ok()?;
    u64::try_from(parsed.timestamp()).ok()
}

fn failed_after_go_ahead(mark: &ActivityMark, facts: &StallFacts) -> bool {
    facts.sent_at.is_none_or(|sent| mark.at >= sent)
}

pub fn stall_action(facts: &StallFacts) -> StallAction {
    let Some(mark) = facts.activity else {
        return StallAction::None;
    };
    if mark.busy() || facts.reported {
        return StallAction::None;
    }
    match &facts.record {
        None if mark.state == ActivityState::Failed && failed_after_go_ahead(&mark, facts) => {
            StallAction::Continue
        }
        None => StallAction::None,
        Some(record)
            if !record.alerted && (mark.at > record.at || went_unanswered(record, facts)) =>
        {
            StallAction::Alert
        }
        Some(_) => StallAction::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(at: u64, state: ActivityState) -> Option<ActivityMark> {
        Some(ActivityMark { at, state })
    }

    fn facts(activity: Option<ActivityMark>) -> StallFacts {
        StallFacts {
            activity,
            reported: false,
            record: None,
            sent_at: None,
            now: 100,
            check_secs: 30,
        }
    }

    fn continued(at: u64) -> Option<AutoContinue> {
        Some(AutoContinue::new(at, REASON_API_ERROR))
    }

    #[test]
    fn a_busy_worker_is_left_alone() {
        for state in [ActivityState::Running, ActivityState::Background] {
            assert_eq!(
                stall_action(&facts(mark(100, state))),
                StallAction::None,
                "{state:?}"
            );
            let with_record = StallFacts {
                record: continued(90),
                ..facts(mark(100, state))
            };
            assert_eq!(stall_action(&with_record), StallAction::None, "{state:?}");
        }
    }

    #[test]
    fn a_worker_without_an_activity_marker_is_left_alone() {
        assert_eq!(stall_action(&facts(None)), StallAction::None);
    }

    #[test]
    fn a_report_since_the_go_ahead_means_no_action() {
        for record in [None, continued(90)] {
            let reported = StallFacts {
                reported: true,
                record,
                ..facts(mark(100, ActivityState::Failed))
            };
            assert_eq!(stall_action(&reported), StallAction::None);
        }
    }

    #[test]
    fn an_idle_failed_worker_gets_one_continue() {
        assert_eq!(
            stall_action(&facts(mark(100, ActivityState::Failed))),
            StallAction::Continue
        );
    }

    #[test]
    fn a_failure_older_than_the_go_ahead_gets_no_continue() {
        let stale = StallFacts {
            sent_at: Some(101),
            ..facts(mark(100, ActivityState::Failed))
        };
        assert_eq!(stall_action(&stale), StallAction::None);
    }

    #[test]
    fn a_failure_at_or_after_the_go_ahead_gets_a_continue() {
        for sent_at in [None, Some(100), Some(50)] {
            let fresh = StallFacts {
                sent_at,
                ..facts(mark(100, ActivityState::Failed))
            };
            assert_eq!(stall_action(&fresh), StallAction::Continue, "{sent_at:?}");
        }
    }

    #[test]
    fn a_plainly_idle_worker_with_no_record_gets_nothing() {
        assert_eq!(
            stall_action(&facts(mark(100, ActivityState::Idle))),
            StallAction::None
        );
    }

    #[test]
    fn idle_or_failed_again_after_a_continue_alerts() {
        for state in [ActivityState::Idle, ActivityState::Failed] {
            let after = StallFacts {
                record: continued(90),
                ..facts(mark(100, state))
            };
            assert_eq!(stall_action(&after), StallAction::Alert, "{state:?}");
        }
    }

    #[test]
    fn an_old_mark_from_before_the_continue_does_not_alert_yet() {
        for state in [ActivityState::Idle, ActivityState::Failed] {
            for marked_at in [89, 90] {
                let before = StallFacts {
                    record: continued(90),
                    ..facts(mark(marked_at, state))
                };
                assert_eq!(stall_action(&before), StallAction::None, "{state:?}");
            }
        }
    }

    #[test]
    fn a_continue_nobody_picked_up_alerts_after_the_grace() {
        let silent = |now| StallFacts {
            record: continued(90),
            now,
            ..facts(mark(80, ActivityState::Failed))
        };
        assert_eq!(stall_action(&silent(149)), StallAction::None);
        assert_eq!(stall_action(&silent(150)), StallAction::Alert);
        assert_eq!(stall_action(&silent(1000)), StallAction::Alert);
    }

    #[test]
    fn a_failure_in_the_same_second_as_the_continue_alerts_after_the_grace() {
        let same = |now| StallFacts {
            record: continued(90),
            now,
            ..facts(mark(90, ActivityState::Failed))
        };
        assert_eq!(stall_action(&same(100)), StallAction::None);
        assert_eq!(stall_action(&same(150)), StallAction::Alert);
    }

    #[test]
    fn a_silent_continue_stays_quiet_once_alerted_or_reported_or_busy() {
        let base = StallFacts {
            record: continued(90),
            now: 1000,
            ..facts(mark(80, ActivityState::Failed))
        };
        let alerted = StallFacts {
            record: continued(90).map(AutoContinue::alerted),
            ..base.clone()
        };
        let reported = StallFacts {
            reported: true,
            ..base.clone()
        };
        let busy = StallFacts {
            activity: mark(95, ActivityState::Running),
            ..base
        };
        for facts in [alerted, reported, busy] {
            assert_eq!(stall_action(&facts), StallAction::None);
        }
    }

    #[test]
    fn the_grace_is_two_check_intervals() {
        assert_eq!(grace_secs(30), 60);
        assert_eq!(grace_secs(u64::MAX), u64::MAX);
    }

    #[test]
    fn a_report_after_the_go_ahead_counts_as_reported() {
        assert!(reported_since_go_ahead(None, None));
        assert!(reported_since_go_ahead(None, Some(10)));
        assert!(!reported_since_go_ahead(Some(50), None));
        assert!(!reported_since_go_ahead(Some(50), Some(49)));
        assert!(reported_since_go_ahead(Some(50), Some(50)));
        assert!(reported_since_go_ahead(Some(50), Some(51)));
    }

    #[test]
    fn a_failure_report_after_the_go_ahead_keeps_an_idle_failed_worker_alone() {
        let failure_report = StallFacts {
            reported: reported_since_go_ahead(Some(50), Some(100)),
            ..facts(mark(99, ActivityState::Failed))
        };
        assert_eq!(stall_action(&failure_report), StallAction::None);
        let old_report = StallFacts {
            reported: reported_since_go_ahead(Some(50), Some(40)),
            ..facts(mark(99, ActivityState::Failed))
        };
        assert_eq!(stall_action(&old_report), StallAction::Continue);
    }

    #[test]
    fn report_secs_reads_rfc3339_and_rejects_garbage() {
        assert_eq!(report_secs("2026-01-01T00:00:10Z"), Some(1_767_225_610));
        assert_eq!(
            report_secs("2026-01-01T01:00:10+01:00"),
            Some(1_767_225_610)
        );
        assert_eq!(report_secs("yesterday"), None);
        assert_eq!(report_secs("1969-12-31T23:59:59Z"), None);
    }

    #[test]
    fn a_written_record_lets_the_nudge_go_without_a_warning() {
        for warned in [false, true] {
            assert_eq!(
                gate_send(true, warned),
                RecordGate {
                    send: true,
                    warn: false
                }
            );
        }
    }

    #[test]
    fn a_failed_record_write_blocks_the_nudge_and_warns_once_per_streak() {
        assert_eq!(
            gate_send(false, false),
            RecordGate {
                send: false,
                warn: true
            }
        );
        assert_eq!(
            gate_send(false, true),
            RecordGate {
                send: false,
                warn: false
            }
        );
    }

    #[test]
    fn a_sent_nudge_is_logged_and_a_failed_one_removes_the_record() {
        assert_eq!(after_send(true), &[ContinueStep::Log]);
        assert_eq!(after_send(false), &[ContinueStep::RemoveRecord]);
    }

    #[test]
    fn an_alerted_record_stays_silent_until_cleared() {
        let alerted = StallFacts {
            record: continued(90).map(AutoContinue::alerted),
            ..facts(mark(100, ActivityState::Failed))
        };
        assert_eq!(stall_action(&alerted), StallAction::None);
    }

    #[test]
    fn the_record_round_trips_and_rejects_garbage() {
        let record = AutoContinue::new(42, REASON_API_ERROR);
        assert_eq!(AutoContinue::parse(&record.render()), Some(record.clone()));
        let alerted = record.alerted();
        assert!(alerted.alerted);
        assert_eq!(AutoContinue::parse(&alerted.render()), Some(alerted));
        assert_eq!(AutoContinue::parse(""), None);
        assert_eq!(AutoContinue::parse("{\"at\":1}"), None);
    }

    #[test]
    fn the_record_renders_the_documented_fields() {
        assert_eq!(
            AutoContinue::new(7, REASON_API_ERROR).render(),
            r#"{"at":7,"reason":"api error","alerted":false}"#
        );
    }

    #[test]
    fn the_log_line_carries_at_worker_harness_and_reason() {
        assert_eq!(
            log_line(7, "GUZ-1", "claude", REASON_API_ERROR),
            r#"{"at":7,"worker":"GUZ-1","harness":"claude","reason":"api error"}"#
        );
    }

    #[test]
    fn paths_live_in_the_store() {
        assert_eq!(
            auto_continued_path(Path::new("/s"), "A"),
            PathBuf::from("/s/auto-continued/A")
        );
        assert_eq!(
            auto_continue_log_path(Path::new("/s")),
            PathBuf::from("/s/auto-continue.jsonl")
        );
    }

    #[test]
    fn the_alert_text_points_at_tmux_only_for_tmux_workers() {
        assert_eq!(
            alert_text("GUZ-1", Some("ralph-x-GUZ-1")),
            "Worker GUZ-1 is still idle with no report after one auto-continue. To look: tmux attach -t ralph-x-GUZ-1"
        );
        assert_eq!(
            alert_text("GUZ-1", None),
            "Worker GUZ-1 is still idle with no report after one auto-continue."
        );
    }

    #[test]
    fn the_continue_text_asks_for_git_status_first() {
        assert!(CONTINUE_TEXT.contains("start with git status"));
    }
}
