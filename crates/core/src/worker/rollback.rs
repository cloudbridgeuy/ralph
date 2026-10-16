/// What one spawn found and did before its harness failed to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnFootprint {
    pub created_worktree: bool,
    pub created_branch: bool,
    /// The harness still runs in the worktree (a tmux session or an owner file).
    pub harness_running: bool,
    /// Commits on the branch past the commit it was created at; `None` when unknown.
    pub commits_past_base: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rollback {
    pub remove_worktree: bool,
    pub delete_branch: bool,
}

/// Undo only what this spawn created, and never a branch that holds work.
pub fn rollback_plan(footprint: &SpawnFootprint) -> Rollback {
    let remove_worktree = footprint.created_worktree && !footprint.harness_running;
    Rollback {
        remove_worktree,
        delete_branch: remove_worktree
            && footprint.created_branch
            && footprint.commits_past_base == Some(0),
    }
}

/// The start error stays first; rollback failures follow it.
pub fn rollback_report(start_error: &str, failures: &[String]) -> String {
    if failures.is_empty() {
        start_error.to_string()
    } else {
        format!("{start_error} (rollback failed: {})", failures.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn created() -> SpawnFootprint {
        SpawnFootprint {
            created_worktree: true,
            created_branch: true,
            harness_running: false,
            commits_past_base: Some(0),
        }
    }

    fn plan(footprint: SpawnFootprint) -> (bool, bool) {
        let rollback = rollback_plan(&footprint);
        (rollback.remove_worktree, rollback.delete_branch)
    }

    #[test]
    fn a_fresh_worktree_and_branch_are_both_removed() {
        assert_eq!(plan(created()), (true, true));
    }

    #[test]
    fn a_reused_worktree_and_branch_are_kept() {
        let reused = SpawnFootprint {
            created_worktree: false,
            created_branch: false,
            ..created()
        };
        assert_eq!(plan(reused), (false, false));
    }

    #[test]
    fn a_fresh_worktree_on_a_reused_branch_keeps_the_branch() {
        let reused_branch = SpawnFootprint {
            created_branch: false,
            ..created()
        };
        assert_eq!(plan(reused_branch), (true, false));
    }

    #[test]
    fn a_branch_with_commits_past_base_is_kept() {
        let worked = SpawnFootprint {
            commits_past_base: Some(2),
            ..created()
        };
        assert_eq!(plan(worked), (true, false));
    }

    #[test]
    fn an_unknown_commit_count_keeps_the_branch() {
        let unknown = SpawnFootprint {
            commits_past_base: None,
            ..created()
        };
        assert_eq!(plan(unknown), (true, false));
    }

    #[test]
    fn a_running_harness_keeps_everything() {
        let running = SpawnFootprint {
            harness_running: true,
            ..created()
        };
        assert_eq!(plan(running), (false, false));
    }

    #[test]
    fn the_branch_is_kept_when_the_worktree_is_reused() {
        let reused_worktree = SpawnFootprint {
            created_worktree: false,
            ..created()
        };
        assert_eq!(plan(reused_worktree), (false, false));
    }

    #[test]
    fn rollback_report_shows_the_start_error_alone_when_rollback_worked() {
        assert_eq!(rollback_report("boom", &[]), "boom");
    }

    #[test]
    fn rollback_report_appends_rollback_failures() {
        let failures = ["worktree busy".to_string(), "branch locked".to_string()];
        assert_eq!(
            rollback_report("boom", &failures),
            "boom (rollback failed: worktree busy; branch locked)"
        );
    }
}
