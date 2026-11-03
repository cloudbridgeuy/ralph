use crate::worker::{
    CheckResult, DriftEntry, DriftOutcome, Report, Verdict, VerifiedAgents, FREE_VERIFY_LIMIT,
    INBOX_DIR,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DRIFT_REPORT_STATUS: &str = "warning";
pub const NO_FAILING_CHECK: &str = "verify spec";
pub const TIME_LIMIT_CHECK: &str = "time limit";

pub fn drift_lock_limit() -> Duration {
    FREE_VERIFY_LIMIT * 2
}

pub fn drift_lock_file(agent: &str) -> String {
    format!("drift-{agent}.lock")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockAction {
    Acquire,
    Busy,
    Replace,
}

pub fn lock_action(age: Option<Duration>) -> LockAction {
    match age {
        None => LockAction::Acquire,
        Some(age) if age < drift_lock_limit() => LockAction::Busy,
        Some(_) => LockAction::Replace,
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
    fn the_lock_file_is_named_per_agent() {
        assert_eq!(drift_lock_file("codex"), "drift-codex.lock");
    }

    #[test]
    fn a_fresh_lock_is_busy_a_missing_one_is_taken_a_stale_one_is_replaced() {
        assert_eq!(lock_action(None), LockAction::Acquire);
        assert_eq!(lock_action(Some(Duration::from_secs(10))), LockAction::Busy);
        assert_eq!(
            lock_action(Some(drift_lock_limit() - Duration::from_secs(1))),
            LockAction::Busy
        );
        assert_eq!(lock_action(Some(drift_lock_limit())), LockAction::Replace);
        assert_eq!(drift_lock_limit(), Duration::from_secs(180));
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
