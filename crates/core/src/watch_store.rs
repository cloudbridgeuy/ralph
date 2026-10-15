use crate::watch::{WatchKind, WatchOutcome, WatchSpec};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const WATCHES_DIR: &str = "watches";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWatch {
    pub id: String,
    pub command: String,
    pub follow_up: Option<String>,
    pub kind: WatchKind,
    pub timeout_secs: Option<u64>,
    pub started_at: DateTime<Utc>,
    pub timeout_deadline: Option<DateTime<Utc>>,
    pub wake_deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreAction {
    Restart(WatchSpec),
    Report {
        spec: WatchSpec,
        outcome: WatchOutcome,
    },
    Discard(String),
}

pub fn file_name(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{hex}.json")
}

pub fn sleep_seconds(command: &str) -> Option<u64> {
    let amount = command.trim().strip_prefix("sleep")?;
    if !amount.starts_with(char::is_whitespace) {
        return None;
    }
    let amount = amount.trim();
    if amount.is_empty() || !amount.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    amount.parse().ok()
}

pub fn sleep_command(seconds: u64) -> String {
    format!("sleep {seconds}")
}

fn deadline_after(start: DateTime<Utc>, after: Duration) -> Option<DateTime<Utc>> {
    let after = ChronoDuration::from_std(after).ok()?;
    start.checked_add_signed(after)
}

fn remaining(deadline: DateTime<Utc>, now: DateTime<Utc>) -> Option<Duration> {
    (deadline - now)
        .to_std()
        .ok()
        .filter(|left| !left.is_zero())
}

fn ceil_secs(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

impl SavedWatch {
    pub fn new(spec: &WatchSpec, started_at: DateTime<Utc>) -> Self {
        Self {
            id: spec.id.clone(),
            command: spec.command.clone(),
            follow_up: spec.follow_up.clone(),
            kind: spec.kind,
            timeout_secs: spec.timeout.map(|limit| limit.as_secs()),
            started_at,
            timeout_deadline: spec
                .timeout
                .and_then(|limit| deadline_after(started_at, limit)),
            wake_deadline: sleep_seconds(&spec.command)
                .and_then(|seconds| deadline_after(started_at, Duration::from_secs(seconds))),
        }
    }

    fn spec(&self) -> WatchSpec {
        WatchSpec {
            id: self.id.clone(),
            command: self.command.clone(),
            follow_up: self.follow_up.clone(),
            kind: self.kind,
            timeout: self.timeout_secs.map(Duration::from_secs),
            limit: self.timeout_secs.map(Duration::from_secs),
        }
    }

    fn restart_spec(&self, now: DateTime<Utc>) -> WatchSpec {
        let command = self
            .wake_deadline
            .and_then(|deadline| remaining(deadline, now))
            .map_or_else(
                || self.command.clone(),
                |left| sleep_command(ceil_secs(left)),
            );
        WatchSpec {
            command,
            timeout: self
                .timeout_deadline
                .and_then(|deadline| remaining(deadline, now)),
            ..self.spec()
        }
    }

    fn timed_out_report(&self) -> RestoreAction {
        let spec = self.spec();
        let limit = spec.limit.or(spec.timeout).unwrap_or_default();
        RestoreAction::Report {
            spec,
            outcome: WatchOutcome::TimedOut(limit),
        }
    }

    fn restore_action(&self, now: DateTime<Utc>, default_timeout: Duration) -> RestoreAction {
        if self.kind != WatchKind::Persistent {
            return RestoreAction::Discard(self.id.clone());
        }
        let woke = self.wake_deadline.filter(|deadline| *deadline <= now);
        let timed_out = self.timeout_deadline.filter(|deadline| *deadline <= now);
        match (woke, timed_out) {
            (Some(wake), Some(limit)) if limit < wake => self.timed_out_report(),
            (Some(_), _) => RestoreAction::Report {
                spec: self.spec(),
                outcome: WatchOutcome::Exited(0),
            },
            (None, Some(_)) => self.timed_out_report(),
            (None, None) => {
                RestoreAction::Restart(self.restart_spec(now).with_default_timeout(default_timeout))
            }
        }
    }
}

pub fn plan_restore(
    saved: &[SavedWatch],
    now: DateTime<Utc>,
    default_timeout: Duration,
) -> Vec<RestoreAction> {
    saved
        .iter()
        .map(|watch| watch.restore_action(now, default_timeout))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: Duration = Duration::from_secs(3600);

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn spec(command: &str, timeout: Option<u64>, kind: WatchKind) -> WatchSpec {
        WatchSpec {
            id: "w".to_string(),
            command: command.to_string(),
            follow_up: Some("next".to_string()),
            kind,
            timeout: timeout.map(Duration::from_secs),
            limit: timeout.map(Duration::from_secs),
        }
    }

    fn persistent(command: &str, timeout: Option<u64>) -> SavedWatch {
        SavedWatch::new(&spec(command, timeout, WatchKind::Persistent), at(0))
    }

    #[test]
    fn restart_of_a_record_without_a_timeout_gets_the_default() {
        let saved = persistent("tail -f log", None);
        let plan = plan_restore(&[saved], at(5), DEFAULT);
        let [RestoreAction::Restart(spec)] = plan.as_slice() else {
            panic!("expected restart, got {plan:?}");
        };
        assert_eq!(spec.timeout, Some(DEFAULT));
        assert_eq!(spec.limit, Some(DEFAULT));
    }

    #[test]
    fn restart_keeps_the_saved_limit_and_the_time_left() {
        let saved = persistent("tail -f log", Some(100));
        let plan = plan_restore(&[saved], at(40), DEFAULT);
        let [RestoreAction::Restart(spec)] = plan.as_slice() else {
            panic!("expected restart, got {plan:?}");
        };
        assert_eq!(spec.timeout, Some(Duration::from_secs(60)));
        assert_eq!(spec.limit, Some(Duration::from_secs(100)));
    }

    #[test]
    fn sleep_seconds_parses_only_exact_sleep_commands() {
        assert_eq!(sleep_seconds("sleep 30"), Some(30));
        assert_eq!(sleep_seconds("  sleep   7  "), Some(7));
        assert_eq!(sleep_seconds("sleep\t5"), Some(5));
        assert_eq!(sleep_seconds("sleep 0"), Some(0));
    }

    #[test]
    fn sleep_seconds_rejects_everything_else() {
        for command in [
            "sleep",
            "sleep 1.5",
            "sleep 5m",
            "sleep -1",
            "sleep +1",
            "sleep 5 && echo hi",
            "sleep5",
            "sleepy 5",
            "echo sleep 5",
            "sleep 99999999999999999999999",
            "",
        ] {
            assert_eq!(sleep_seconds(command), None, "{command}");
        }
    }

    #[test]
    fn sleep_command_round_trips() {
        assert_eq!(sleep_seconds(&sleep_command(12)), Some(12));
    }

    #[test]
    fn file_name_is_stable_and_safe() {
        assert_eq!(file_name("a/b"), file_name("a/b"));
        assert_ne!(file_name("a"), file_name("b"));
        let name = file_name("../x y");
        assert!(name.ends_with(".json"));
        assert!(name
            .trim_end_matches(".json")
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn new_computes_deadlines_from_start() {
        let saved = persistent("sleep 10", Some(60));
        assert_eq!(saved.timeout_deadline, Some(at(60)));
        assert_eq!(saved.wake_deadline, Some(at(10)));
        let plain = persistent("gh run watch 1", None);
        assert_eq!(plain.timeout_deadline, None);
        assert_eq!(plain.wake_deadline, None);
    }

    #[test]
    fn new_ignores_deadlines_that_overflow() {
        let saved = persistent("sleep 18446744073709551615", Some(u64::MAX));
        assert_eq!(saved.timeout_deadline, None);
        assert_eq!(saved.wake_deadline, None);
    }

    #[test]
    fn restart_rewrites_sleep_with_remaining_seconds_rounded_up() {
        let saved = persistent("sleep 10", None);
        let plan = plan_restore(
            &[saved],
            at(0) + ChronoDuration::milliseconds(3500),
            DEFAULT,
        );
        let [RestoreAction::Restart(spec)] = plan.as_slice() else {
            panic!("{plan:?}");
        };
        assert_eq!(spec.command, "sleep 7");
        assert_eq!(spec.timeout, Some(DEFAULT));
    }

    #[test]
    fn restart_shrinks_the_timeout_and_keeps_other_commands() {
        let saved = persistent("gh run watch 1", Some(60));
        let plan = plan_restore(&[saved], at(20), DEFAULT);
        let [RestoreAction::Restart(spec)] = plan.as_slice() else {
            panic!("{plan:?}");
        };
        assert_eq!(spec.command, "gh run watch 1");
        assert_eq!(spec.timeout, Some(Duration::from_secs(40)));
        assert_eq!(spec.follow_up.as_deref(), Some("next"));
        assert_eq!(spec.kind, WatchKind::Persistent);
    }

    #[test]
    fn restart_twice_uses_the_original_deadlines() {
        let saved = persistent("sleep 10", Some(100));
        let first = plan_restore(std::slice::from_ref(&saved), at(3), DEFAULT);
        let second = plan_restore(std::slice::from_ref(&saved), at(6), DEFAULT);
        let commands: Vec<String> = [first, second]
            .into_iter()
            .flatten()
            .map(|action| match action {
                RestoreAction::Restart(spec) => spec.command,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(commands, ["sleep 7", "sleep 4"]);
    }

    #[test]
    fn passed_wake_deadline_fires_now() {
        let saved = persistent("sleep 10", Some(100));
        let plan = plan_restore(&[saved], at(10), DEFAULT);
        assert!(matches!(
            plan.as_slice(),
            [RestoreAction::Report {
                outcome: WatchOutcome::Exited(0),
                ..
            }]
        ));
    }

    #[test]
    fn passed_timeout_deadline_times_out_now() {
        let saved = persistent("gh run watch 1", Some(30));
        let plan = plan_restore(&[saved], at(31), DEFAULT);
        let [RestoreAction::Report { spec, outcome }] = plan.as_slice() else {
            panic!("{plan:?}");
        };
        assert_eq!(spec.timeout, Some(Duration::from_secs(30)));
        assert_eq!(*outcome, WatchOutcome::TimedOut(Duration::from_secs(30)));
    }

    #[test]
    fn a_timed_out_restore_reports_the_original_limit_not_the_time_left() {
        let saved = persistent("tail -f log", Some(100));
        let plan = plan_restore(&[saved], at(250), DEFAULT);
        let [RestoreAction::Report { outcome, .. }] = plan.as_slice() else {
            panic!("{plan:?}");
        };
        assert_eq!(*outcome, WatchOutcome::TimedOut(Duration::from_secs(100)));
    }

    #[test]
    fn earlier_passed_deadline_wins() {
        let timeout_first = persistent("sleep 50", Some(20));
        let wake_first = persistent("sleep 20", Some(50));
        let plan = plan_restore(&[timeout_first, wake_first], at(60), DEFAULT);
        assert!(matches!(
            plan.as_slice(),
            [
                RestoreAction::Report {
                    outcome: WatchOutcome::TimedOut(_),
                    ..
                },
                RestoreAction::Report {
                    outcome: WatchOutcome::Exited(0),
                    ..
                }
            ]
        ));
    }

    #[test]
    fn normal_records_are_discarded_never_restored() {
        let saved = SavedWatch::new(&spec("sleep 10", None, WatchKind::Normal), at(0));
        let plan = plan_restore(&[saved], at(1), DEFAULT);
        assert_eq!(plan, [RestoreAction::Discard("w".to_string())]);
    }

    #[test]
    fn untimed_non_sleep_watch_restarts_with_the_default_timeout() {
        let saved = persistent("tail -f log", None);
        let plan = plan_restore(&[saved], at(1000), DEFAULT);
        let [RestoreAction::Restart(spec)] = plan.as_slice() else {
            panic!("{plan:?}");
        };
        assert_eq!(spec.command, "tail -f log");
        assert_eq!(spec.timeout, Some(DEFAULT));
    }

    #[test]
    fn saved_watch_survives_json() {
        let saved = persistent("sleep 10", Some(60));
        let json = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<SavedWatch>(&json).unwrap(), saved);
    }
}
