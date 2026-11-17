use super::*;
use std::path::PathBuf;

const COMMAND: &str = "cargo test -p core";
const PORCELAIN: &str = "worktree /repo\nHEAD aaa\nbranch refs/heads/main\n\nworktree /repo/.worktrees/E1\nHEAD bbb\nbranch refs/heads/worker/E1\n";

fn verifying_plan() -> RunPlan {
    let mut json = plan_json();
    json["steps"][0]["criterion"]["verify"] = serde_json::json!(COMMAND);
    parse_run(&json, limits()).unwrap()
}

fn verifying_progress(step: usize) -> Progress {
    Progress {
        verify: VerifyMode::Enforce,
        verifying: true,
        ..at(step)
    }
}

#[test]
fn the_verify_worker_is_the_worker_of_a_step_that_is_verifying() {
    assert_eq!(
        verify_worker(&verifying_plan(), verifying_progress(0)),
        Some("E1")
    );
}

#[test]
fn no_verify_worker_when_the_step_is_not_verifying() {
    let progress = Progress {
        verifying: false,
        ..verifying_progress(0)
    };
    assert_eq!(verify_worker(&verifying_plan(), progress), None);
}

#[test]
fn no_verify_worker_on_a_step_that_is_not_a_worker_step() {
    assert_eq!(
        verify_worker(&verifying_plan(), verifying_progress(2)),
        None
    );
    assert_eq!(
        verify_worker(&verifying_plan(), verifying_progress(9)),
        None
    );
}

#[test]
fn a_verify_spec_runs_in_the_worker_worktree_under_the_check_id() {
    let spec = verify_spec(0, COMMAND, PathBuf::from("/repo/.worktrees/E1"));
    assert_eq!(spec.cwd, Some(PathBuf::from("/repo/.worktrees/E1")));
    assert_eq!(WatchSpec { cwd: None, ..spec }, check_spec(0, COMMAND));
}

#[test]
fn a_plain_check_spec_has_no_cwd() {
    assert_eq!(check_spec(0, COMMAND).cwd, None);
}

#[test]
fn a_check_without_a_verify_worker_is_the_plain_check() {
    assert_eq!(
        resolve_check(2, COMMAND, None, None),
        Ok(check_spec(2, COMMAND))
    );
}

#[test]
fn a_verify_check_runs_in_the_worker_worktree() {
    let spec = resolve_check(0, COMMAND, Some("E1"), Some(PORCELAIN)).unwrap();
    assert_eq!(spec.cwd, Some(PathBuf::from("/repo/.worktrees/E1")));
    assert_eq!(spec.command, COMMAND);
}

#[test]
fn a_verify_check_stops_when_the_porcelain_has_no_worktree_for_the_worker() {
    for porcelain in [Some(PORCELAIN), Some(""), None] {
        let stop = resolve_check(0, COMMAND, Some("E2"), porcelain).unwrap_err();
        match stop {
            StopReason::CheckNotStarted { step, error } => {
                assert_eq!(step, 1);
                assert!(error.contains("E2"), "{error}");
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn a_verify_check_stops_when_the_worktree_list_is_unavailable() {
    assert!(matches!(
        resolve_check(0, COMMAND, Some("E1"), None),
        Err(StopReason::CheckNotStarted { step: 1, .. })
    ));
}
