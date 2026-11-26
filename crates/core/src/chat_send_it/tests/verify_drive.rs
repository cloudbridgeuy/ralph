use super::*;
use std::path::PathBuf;
use std::time::Duration;

const VERIFY: &str = "cargo test -p core";
const WATCH_ID: &str = "watch:send-it-1";

fn verifying_plan() -> RunPlan {
    let mut json = plan_json();
    json["steps"][0]["criterion"]["verify"] = serde_json::json!(VERIFY);
    parse_run(&json, limits()).unwrap()
}

fn with_mode(step: usize, mode: VerifyMode) -> Progress {
    Progress {
        verify: mode,
        ..at(step)
    }
}

fn verifying(mode: VerifyMode) -> Progress {
    Progress {
        verifying: true,
        ..with_mode(0, mode)
    }
}

fn done() -> Report {
    worker_report("E1", "done")
}

fn failed_check(driving: &Driving, file: &RunFile, outcome: &WatchOutcome) -> Ticked {
    let output = Report {
        text: "Watch done.\n\nOutput:\nassertion failed".to_string(),
        ..worker_report(WATCH_ID, "done")
    };
    settle_check(driving, Some(file), outcome, &[output], TRUST)
}

fn waiting_on_check(mode: VerifyMode, file: &RunFile) -> Driving {
    Driving {
        progress: verifying(mode),
        ..driving(0, Awaiting::Report(WATCH_ID.to_string()), Run::default())
    }
    .with_hash_of(file)
}

trait WithHash {
    fn with_hash_of(self, file: &RunFile) -> Self;
}

impl WithHash for Driving {
    fn with_hash_of(self, file: &RunFile) -> Self {
        Driving {
            hash: plan_hash(&file.plan),
            ..self
        }
    }
}

#[test]
fn a_done_report_with_a_verify_command_asks_for_the_check() {
    for mode in [VerifyMode::Observe, VerifyMode::Enforce] {
        let (progress, next) = advance(
            &verifying_plan(),
            with_mode(0, mode),
            Event::Report(&done()),
        );
        assert_eq!(next, Next::Check(VERIFY.to_string()));
        assert_eq!(progress, verifying(mode));
    }
}

#[test]
fn a_passing_verify_moves_to_the_next_step_and_keeps_the_mode() {
    let (progress, next) = advance(
        &verifying_plan(),
        verifying(VerifyMode::Enforce),
        Event::Watched(&WatchOutcome::Exited(0)),
    );
    assert_eq!(progress, with_mode(1, VerifyMode::Enforce));
    assert_eq!(next, Next::Watch("gh pr checks --watch".to_string()));
}

#[test]
fn observe_moves_on_when_verify_fails() {
    let (progress, next) = advance(
        &verifying_plan(),
        verifying(VerifyMode::Observe),
        Event::Watched(&WatchOutcome::Exited(1)),
    );
    assert_eq!(progress, with_mode(1, VerifyMode::Observe));
    assert_eq!(next, Next::Watch("gh pr checks --watch".to_string()));
}

#[test]
fn enforce_stops_when_verify_fails() {
    let (progress, next) = advance(
        &verifying_plan(),
        verifying(VerifyMode::Enforce),
        Event::Watched(&WatchOutcome::Exited(1)),
    );
    assert_eq!(progress, verifying(VerifyMode::Enforce));
    assert_eq!(next, Next::Stop(StopReason::check_failed(1)));
}

#[test]
fn off_and_a_missing_verify_move_on_at_once() {
    let off = advance(
        &verifying_plan(),
        with_mode(0, VerifyMode::Off),
        Event::Report(&done()),
    );
    let none = advance(
        &plan(),
        with_mode(0, VerifyMode::Enforce),
        Event::Report(&done()),
    );
    for (progress, next) in [off, none] {
        assert!(!progress.verifying);
        assert_eq!(progress.step, 1);
        assert_eq!(next, Next::Watch("gh pr checks --watch".to_string()));
    }
}

#[test]
fn a_report_that_is_not_done_stays_unmet_before_any_verify() {
    let (_, next) = advance(
        &verifying_plan(),
        with_mode(0, VerifyMode::Enforce),
        Event::Report(&worker_report("E1", "check")),
    );
    assert_eq!(next, Next::Stop(StopReason::Unmet { step: 1 }));
}

#[test]
fn observe_logs_a_would_block_notice_with_the_detail_and_moves_on() {
    let file = approved(verifying_plan());
    let ticked = failed_check(
        &waiting_on_check(VerifyMode::Observe, &file),
        &file,
        &WatchOutcome::Exited(2),
    );
    assert_eq!(ticked.driving.progress.step, 1);
    assert_eq!(ticked.next, Next::Watch("gh pr checks --watch".to_string()));
    let notice = ticked.notice.unwrap();
    assert_eq!(
        notice,
        "send-it: step 1 would block: verify failed: exited with code 2; last output: assertion failed"
    );
    assert_eq!(
        ticked.met.as_deref(),
        Some("send-it: step 1/4 met (worker E1 reports done, then `cargo test -p core` exits 0)")
    );
}

#[test]
fn observe_logs_a_would_block_notice_on_a_timeout() {
    let file = approved(verifying_plan());
    let ticked = settle_check(
        &waiting_on_check(VerifyMode::Observe, &file),
        Some(&file),
        &WatchOutcome::TimedOut(Duration::from_secs(60)),
        &[],
        TRUST,
    );
    assert!(ticked
        .notice
        .unwrap()
        .contains("would block: verify failed: timed out after 60s"),);
}

#[test]
fn observe_logs_no_notice_when_verify_passes() {
    let file = approved(verifying_plan());
    let ticked = settle_check(
        &waiting_on_check(VerifyMode::Observe, &file),
        Some(&file),
        &WatchOutcome::Exited(0),
        &[],
        TRUST,
    );
    assert_eq!(
        ticked.met.as_deref(),
        Some("send-it: step 1/4 met (worker E1 reports done, then `cargo test -p core` exits 0)")
    );
    assert_eq!(ticked.notice, None);
}

#[test]
fn enforce_stops_with_the_check_detail() {
    let file = approved(verifying_plan());
    let ticked = failed_check(
        &waiting_on_check(VerifyMode::Enforce, &file),
        &file,
        &WatchOutcome::Exited(2),
    );
    assert_eq!(
        ticked.next,
        Next::Stop(StopReason::CheckFailed {
            step: 1,
            detail: Some("exited with code 2; last output: assertion failed".to_string()),
        })
    );
    assert_eq!(ticked.met, None);
}

const PORCELAIN: &str = "worktree /repo\nHEAD aaa\nbranch refs/heads/main\n\nworktree /repo/.worktrees/E1\nHEAD bbb\nbranch refs/heads/worker/E1\n";

#[test]
fn verify_cwd_finds_the_worker_worktree() {
    assert_eq!(
        verify_cwd(PORCELAIN, "E1"),
        Some(PathBuf::from("/repo/.worktrees/E1"))
    );
}

#[test]
fn verify_cwd_is_none_without_a_worktree() {
    assert_eq!(verify_cwd(PORCELAIN, "E2"), None);
    assert_eq!(verify_cwd("", "E1"), None);
}
