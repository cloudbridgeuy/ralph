use crate::worker::{
    CheckResult, DriftEntry, DriftOutcome, Report, Verdict, VerifiedAgents, FREE_VERIFY_LIMIT,
    INBOX_DIR,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DRIFT_REPORT_STATUS: &str = "warning";
pub const NO_FAILING_CHECK: &str = "verify spec";
pub const TIME_LIMIT_CHECK: &str = "time limit";

pub const WATCHDOG_MARGIN: Duration = Duration::from_secs(30);

pub fn drift_watchdog_limit() -> Duration {
    FREE_VERIFY_LIMIT + WATCHDOG_MARGIN
}

pub fn drift_lock_limit() -> Duration {
    FREE_VERIFY_LIMIT * 2
}

pub fn drift_lock_file(agent: &str) -> String {
    format!("drift-{agent}.lock")
}

pub const LOCK_PID_REUSE_LIMIT: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockOwner {
    pub pid: u32,
    pub token: String,
}

pub fn format_lock(owner: &LockOwner) -> String {
    format!("{} {}", owner.pid, owner.token)
}

pub fn parse_lock(text: &str) -> Option<LockOwner> {
    let mut fields = text.split_whitespace();
    let pid: u32 = fields.next()?.parse().ok()?;
    let token = fields.next()?;
    if fields.next().is_some() || pid == 0 || i32::try_from(pid).is_err() {
        return None;
    }
    Some(LockOwner {
        pid,
        token: token.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockDecision {
    Acquire,
    Busy,
    TakeOver { expected: Option<String> },
}

pub fn lock_decision(
    age: Option<Duration>,
    owner: Option<&LockOwner>,
    owner_alive: bool,
) -> LockDecision {
    let Some(age) = age else {
        return LockDecision::Acquire;
    };
    let expected = owner.map(|owner| owner.token.clone());
    let stale = match owner {
        Some(_) if owner_alive => age > LOCK_PID_REUSE_LIMIT,
        Some(_) => true,
        None => age >= drift_lock_limit(),
    };
    if stale {
        LockDecision::TakeOver { expected }
    } else {
        LockDecision::Busy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverStep {
    Remove,
    Restore,
}

pub fn takeover_step(expected: Option<&str>, moved: Option<&str>) -> TakeoverStep {
    if expected == moved {
        TakeoverStep::Remove
    } else {
        TakeoverStep::Restore
    }
}

pub fn owns_lock(token: &str, content: &str) -> bool {
    parse_lock(content).is_some_and(|owner| owner.token == token)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinishPlan {
    pub save_verified: bool,
    pub record_drift: bool,
}

pub fn finish_plan(claimed: bool, passed: bool) -> FinishPlan {
    FinishPlan {
        save_verified: claimed && passed,
        record_drift: claimed,
    }
}

pub fn record_after_post(outcome: &DriftOutcome, posted: bool, save_failed: bool) -> bool {
    match outcome {
        DriftOutcome::Passed => !save_failed,
        DriftOutcome::Failed { .. } => posted,
    }
}

pub fn failing_checks(results: &[CheckResult], passed: bool) -> Vec<String> {
    let failed: Vec<String> = results
        .iter()
        .filter(|result| result.verdict == Verdict::Fail)
        .map(|result| result.name.to_string())
        .collect();
    match (passed, failed.is_empty()) {
        (true, _) => Vec::new(),
        (false, true) => vec![NO_FAILING_CHECK.to_string()],
        (false, false) => failed,
    }
}

pub fn drift_entry(
    agent: &str,
    version: &str,
    results: &[CheckResult],
    passed: bool,
    at: &str,
) -> DriftEntry {
    DriftEntry {
        agent: agent.to_string(),
        version: version.to_string(),
        outcome: if passed {
            DriftOutcome::Passed
        } else {
            DriftOutcome::Failed {
                checks: failing_checks(results, false),
            }
        },
        at: at.to_string(),
    }
}

pub fn last_verified_version(verified: &VerifiedAgents, agent: &str) -> Option<String> {
    verified
        .agents
        .iter()
        .rev()
        .find(|entry| entry.agent == agent)
        .map(|entry| entry.version.clone())
}

pub fn drift_alert(
    agent: &str,
    old: Option<&str>,
    new: &str,
    checks: &[String],
    at: &str,
) -> Report {
    Report {
        worker: format!("drift-{agent}"),
        at: at.to_string(),
        status: DRIFT_REPORT_STATUS.to_string(),
        text: format!(
            "{agent} {new} is installed but failed `ralph worker verify {agent}`. Last verified version: {}. Failing checks: {}. Running {agent} workers keep the old binary until restart.",
            old.unwrap_or("none"),
            checks.join(", ")
        ),
    }
}

pub fn known_fail_warning(agent: &str, version: &str, checks: &[String]) -> String {
    format!(
        "{agent} {version} failed `ralph worker verify {agent}` ({}); the worker may not work with this version. Run `ralph worker verify {agent}` to try again.",
        checks.join(", ")
    )
}

pub fn store_inbox(warning_file: &Path) -> Option<PathBuf> {
    warning_file
        .parent()?
        .parent()
        .map(|store| store.join(INBOX_DIR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::verified_entry;

    fn checks(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_watchdog_outlasts_the_verify_budget_and_the_stale_lock_outlasts_the_watchdog() {
        assert_eq!(
            drift_watchdog_limit(),
            FREE_VERIFY_LIMIT + Duration::from_secs(30)
        );
        assert!(drift_lock_limit() > drift_watchdog_limit());
    }

    #[test]
    fn a_failure_is_recorded_only_after_its_alert_posted_and_a_pass_only_after_its_save() {
        let failed = DriftOutcome::Failed {
            checks: checks(&["spawn"]),
        };
        assert!(record_after_post(&failed, true, false));
        assert!(!record_after_post(&failed, false, false));
        assert!(record_after_post(&DriftOutcome::Passed, false, false));
        assert!(record_after_post(&DriftOutcome::Passed, true, false));
        assert!(!record_after_post(&DriftOutcome::Passed, false, true));
        assert!(!record_after_post(&DriftOutcome::Passed, true, true));
    }

    #[test]
    fn only_the_run_that_claimed_the_outcome_writes_anything() {
        assert_eq!(
            finish_plan(true, true),
            FinishPlan {
                save_verified: true,
                record_drift: true
            }
        );
        assert_eq!(
            finish_plan(true, false),
            FinishPlan {
                save_verified: false,
                record_drift: true
            }
        );
        for passed in [true, false] {
            assert_eq!(
                finish_plan(false, passed),
                FinishPlan {
                    save_verified: false,
                    record_drift: false
                }
            );
        }
    }

    #[test]
    fn the_lock_file_is_named_per_agent() {
        assert_eq!(drift_lock_file("codex"), "drift-codex.lock");
    }

    fn owner(pid: u32, token: &str) -> LockOwner {
        LockOwner {
            pid,
            token: token.to_string(),
        }
    }

    #[test]
    fn a_lock_holds_the_owner_pid_and_token_and_reads_back() {
        assert_eq!(format_lock(&owner(42, "ab-cd")), "42 ab-cd");
        assert_eq!(parse_lock("42 ab-cd"), Some(owner(42, "ab-cd")));
        assert_eq!(parse_lock("42 ab-cd\n"), Some(owner(42, "ab-cd")));
    }

    #[test]
    fn a_garbled_lock_has_no_owner() {
        for text in [
            "",
            "42",
            "x ab",
            "0 ab",
            "2147483648 ab",
            "-3 ab",
            "42 ",
            "42 a b",
        ] {
            assert_eq!(parse_lock(text), None, "{text:?}");
        }
    }

    #[test]
    fn no_lock_is_taken() {
        assert_eq!(lock_decision(None, None, false), LockDecision::Acquire);
    }

    #[test]
    fn a_live_owner_is_never_taken_over_by_age_alone_until_the_pid_reuse_limit() {
        let live = owner(7, "t");
        for secs in [0, 179, 180, 3600] {
            assert_eq!(
                lock_decision(Some(Duration::from_secs(secs)), Some(&live), true),
                LockDecision::Busy,
                "{secs}"
            );
        }
        assert_eq!(
            lock_decision(
                Some(LOCK_PID_REUSE_LIMIT + Duration::from_secs(1)),
                Some(&live),
                true
            ),
            LockDecision::TakeOver {
                expected: Some("t".to_string())
            }
        );
    }

    #[test]
    fn a_dead_owner_is_taken_over_at_any_age_and_names_the_token_it_read() {
        assert_eq!(
            lock_decision(Some(Duration::from_secs(1)), Some(&owner(7, "t")), false),
            LockDecision::TakeOver {
                expected: Some("t".to_string())
            }
        );
    }

    #[test]
    fn an_unreadable_owner_is_taken_over_only_when_the_lock_is_stale() {
        assert_eq!(
            lock_decision(
                Some(drift_lock_limit() - Duration::from_secs(1)),
                None,
                false
            ),
            LockDecision::Busy
        );
        assert_eq!(
            lock_decision(Some(drift_lock_limit()), None, false),
            LockDecision::TakeOver { expected: None }
        );
        assert_eq!(drift_lock_limit(), Duration::from_secs(180));
    }

    #[test]
    fn a_moved_lock_is_removed_only_when_it_holds_the_token_that_was_read() {
        assert_eq!(takeover_step(Some("a"), Some("a")), TakeoverStep::Remove);
        assert_eq!(takeover_step(None, None), TakeoverStep::Remove);
        assert_eq!(takeover_step(Some("a"), Some("b")), TakeoverStep::Restore);
        assert_eq!(takeover_step(Some("a"), None), TakeoverStep::Restore);
        assert_eq!(takeover_step(None, Some("b")), TakeoverStep::Restore);
    }

    #[test]
    fn a_run_releases_only_a_lock_that_holds_its_own_token() {
        assert!(owns_lock("t", "42 t"));
        assert!(!owns_lock("t", "42 other"));
        assert!(!owns_lock("t", ""));
        assert!(!owns_lock("t", "garbled"));
    }

    #[test]
    fn only_failed_checks_are_named() {
        let results = [
            CheckResult::pass("initialize", "ok"),
            CheckResult::fail("session/new", "boom"),
            CheckResult::advisory("cancel", Verdict::Unknown, "n/a"),
            CheckResult::fail("login", "no"),
        ];
        assert_eq!(
            failing_checks(&results, false),
            checks(&["session/new", "login"])
        );
        assert_eq!(
            failing_checks(&results[..1], false),
            checks(&[NO_FAILING_CHECK])
        );
        assert_eq!(failing_checks(&results[..1], true), Vec::<String>::new());
    }

    #[test]
    fn a_passed_spec_maps_to_a_passed_entry() {
        let results = [CheckResult::pass("initialize", "ok")];
        assert_eq!(
            drift_entry("codex", "0.170.0", &results, true, "2026-11-03T00:00:00Z"),
            DriftEntry {
                agent: "codex".to_string(),
                version: "0.170.0".to_string(),
                outcome: DriftOutcome::Passed,
                at: "2026-11-03T00:00:00Z".to_string(),
            }
        );
    }

    #[test]
    fn a_failed_spec_maps_to_a_failed_entry_with_the_check_names() {
        let results = [
            CheckResult::pass("initialize", "ok"),
            CheckResult::fail("thread/start", "boom"),
        ];
        assert_eq!(
            drift_entry("codex", "0.170.0", &results, false, "t").outcome,
            DriftOutcome::Failed {
                checks: checks(&["thread/start"])
            }
        );
    }

    #[test]
    fn the_last_verified_version_is_the_newest_entry_of_that_agent() {
        let verified = VerifiedAgents::default()
            .record(verified_entry("codex", "0.160.0", false, "d"))
            .record(verified_entry("cursor", "2026.10.1", false, "d"))
            .record(verified_entry("codex", "0.165.0", false, "d"));
        assert_eq!(
            last_verified_version(&verified, "codex"),
            Some("0.165.0".to_string())
        );
        assert_eq!(last_verified_version(&verified, "opencode"), None);
    }

    #[test]
    fn the_alert_names_harness_versions_checks_and_the_restart_rule() {
        let report = drift_alert(
            "codex",
            Some("0.165.0"),
            "0.170.0",
            &checks(&["initialize", "thread/start"]),
            "2026-11-03T00:00:00Z",
        );
        assert_eq!(report.worker, "drift-codex");
        assert_eq!(report.status, DRIFT_REPORT_STATUS);
        assert_eq!(report.at, "2026-11-03T00:00:00Z");
        assert_eq!(
            report.text,
            "codex 0.170.0 is installed but failed `ralph worker verify codex`. Last verified version: 0.165.0. Failing checks: initialize, thread/start. Running codex workers keep the old binary until restart."
        );
    }

    #[test]
    fn the_alert_says_none_when_nothing_was_verified() {
        let report = drift_alert("codex", None, "0.170.0", &checks(&["time limit"]), "t");
        assert!(report
            .text
            .contains("Last verified version: none. Failing checks: time limit."));
    }

    #[test]
    fn the_known_fail_warning_names_the_failed_checks() {
        assert_eq!(
            known_fail_warning("codex", "0.170.0", &checks(&["initialize", "login"])),
            "codex 0.170.0 failed `ralph worker verify codex` (initialize, login); the worker may not work with this version. Run `ralph worker verify codex` to try again."
        );
    }

    #[test]
    fn the_inbox_sits_next_to_the_handoff_dirs_of_the_store() {
        assert_eq!(
            store_inbox(Path::new("/s/errors/GUZ-1.warning")),
            Some(PathBuf::from("/s").join(INBOX_DIR))
        );
        assert_eq!(store_inbox(Path::new("GUZ-1.warning")), None);
    }
}
