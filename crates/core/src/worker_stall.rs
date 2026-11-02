use crate::worker::{ActivityMark, ActivityState};
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallAction {
    None,
    Continue,
    Alert,
}

pub fn stall_action(facts: &StallFacts) -> StallAction {
    let Some(mark) = facts.activity else {
        return StallAction::None;
    };
    if mark.busy() || facts.reported {
        return StallAction::None;
    }
    match &facts.record {
        None if mark.state == ActivityState::Failed => StallAction::Continue,
        None => StallAction::None,
        Some(record) if !record.alerted && mark.at > record.at => StallAction::Alert,
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
