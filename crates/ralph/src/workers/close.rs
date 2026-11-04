use super::{
    branch_exists, clear_marker, git_value, locate_worktree, repo_root, run, stdout_line,
    unset_legacy_base, Workers,
};
use crate::claude_cleanup::cleanup_worktree_leftovers;
use crate::config::AppConfig;
use crate::cursor_cleanup::cleanup_cursor_leftovers;
use crate::outcome_log;
use crate::warn::warn_if_err;
use crate::worker_changes::WorkerChanges;
use ralph_core::claude_leftovers::{
    cleans_claude_leftovers, spawned_claude_files, trust_lock_path, CLAUDE_CONFIG_DIR_ENV,
};
use ralph_core::cursor_leftovers::cleans_cursor_leftovers;
use ralph_core::worker::{
    base_branch, branch_name, close_clear_paths, close_event, close_plan, close_remove_paths,
    decode_spawn_record, project_slug, spawn_record_path, CloseFacts, CloseState, CloseStep,
    HandoffFiles, HarnessId, Owner, OPENCODE_CONFIG_FILE,
};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

impl Workers {
    pub fn close(&self, id: &str, abandon: bool, force: bool) -> io::Result<String> {
        let repo = repo_root()?;
        self.refuse_busy_headless(id, true)?;
        let owner = self.owner(&project_slug(&repo), id);
        let foreign_owner = matches!(owner, Owner::Foreign(_));
        let config = AppConfig::load().map_err(|e| e.to_string());
        let (worktree, worktree_dir) =
            locate_worktree(&repo, id, self.worktree_dir.as_deref(), config.as_ref())?;
        let branch = branch_name(id);
        let dirty_worktree = if worktree.join(".git").exists() {
            Some(
                !run(Command::new("git")
                    .arg("-C")
                    .arg(&worktree)
                    .args(["status", "--porcelain"]))?
                .stdout
                .is_empty(),
            )
        } else {
            None
        };
        let base = base_branch(
            config
                .as_ref()
                .map(|config| config.chat.worker_base.as_deref())
                .map_err(String::as_str),
            git_value(&repo, &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"])?.as_deref(),
        );
        let unmerged_commits = if branch_exists(&repo, &branch)? {
            let count = stdout_line(run(Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["rev-list", "--count"])
                .arg(format!("{}..{branch}", base.branch)))?);
            Some(count.parse().map_err(io::Error::other)?)
        } else {
            None
        };
        let kind = if HandoffFiles::cursor(&self.store_dir, id).owner.exists() {
            HarnessId::Cursor
        } else if self.codex_running(id) {
            HarnessId::Codex
        } else if self.claude_headless_running(id) {
            HarnessId::ClaudeHeadless
        } else if self.opencode_http(id) {
            HarnessId::Opencode
        } else {
            HarnessId::Claude
        };
        let opencode = kind == HarnessId::Opencode || worktree.join(OPENCODE_CONFIG_FILE).exists();
        let state = CloseState {
            id: id.to_string(),
            owner,
            dirty_worktree,
            base: base.branch.clone(),
            unmerged_commits,
            opencode,
            markers: close_clear_paths(&self.store_dir, id)
                .iter()
                .chain(&close_remove_paths(&self.store_dir, id))
                .any(|marker| marker.exists()),
            kind,
        };
        let with_warning = |text: String| {
            [&base.warning, &worktree_dir.warning]
                .into_iter()
                .flatten()
                .fold(text, |text, warning| format!("{text} {warning}"))
        };
        let steps = close_plan(&state, abandon, force)
            .map_err(|refusal| io::Error::other(with_warning(refusal)))?;
        let spawn_at = fs::read_to_string(spawn_record_path(&self.store_dir, id))
            .ok()
            .and_then(|raw| decode_spawn_record(&raw))
            .map(|record| record.at);
        let real_worktree = fs::canonicalize(&worktree).unwrap_or_else(|_| worktree.clone());
        let changes = WorkerChanges::open(&self.site(id, &worktree));
        for step in &steps {
            self.close_step(step, &repo, &worktree, id).map_err(|e| {
                io::Error::other(format!("closing worker {id} stopped at {step:?}: {e}"))
            })?;
        }
        if let Some(changes) = &changes {
            warn_if_err(changes.clear(), "Failed to clear worker snapshots");
        }
        let unset_warning = unset_legacy_base(&repo, id);
        let files = HandoffFiles::cursor(&self.store_dir, id);
        clear_marker(&files.session)?;
        clear_marker(&files.interrupt_result)?;
        clear_marker(&HandoffFiles::codex(&self.store_dir, id).session)?;
        if steps == [CloseStep::ClearMarkers] {
            return Ok(unset_warning.into_iter().fold(
                with_warning(format!(
                    "Closed worker {id}: only its markers were left; removed them."
                )),
                |text, warning| format!("{text} {warning}"),
            ));
        }
        outcome_log::record(&self.store_dir, id, |key| {
            close_event(&CloseFacts {
                key,
                abandon,
                force,
                foreign_owner,
                commit_count: unmerged_commits.and_then(|count| u64::try_from(count).ok()),
                dirty: dirty_worktree,
                spawn_at,
            })
        });
        let note = if opencode {
            " Evicted its opencode location."
        } else {
            ""
        };
        let leftovers = if cleans_claude_leftovers(&state, &steps) {
            dirs::home_dir()
                .map(|home| {
                    let lock = trust_lock_path(dirs::data_dir().as_deref());
                    let inherited = std::env::var_os(CLAUDE_CONFIG_DIR_ENV).map(PathBuf::from);
                    let files = spawned_claude_files(
                        config
                            .as_ref()
                            .ok()
                            .and_then(|config| config.claude.config_dir.as_deref()),
                        inherited.as_deref(),
                        &home,
                        &worktree,
                    );
                    cleanup_worktree_leftovers(&files, lock.as_deref(), &worktree)
                })
                .unwrap_or_default()
        } else if cleans_cursor_leftovers(&state, &steps) {
            let porcelain = run(Command::new("git").arg("-C").arg(&repo).args([
                "worktree",
                "list",
                "--porcelain",
            ]))
            .ok()
            .map(|listing| String::from_utf8_lossy(&listing.stdout).into_owned());
            dirs::home_dir()
                .map(|home| {
                    cleanup_cursor_leftovers(&home, &worktree, &real_worktree, porcelain.as_deref())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(unset_warning.into_iter().chain(leftovers).fold(
            with_warning(format!(
                "Closed worker {id}: removed its sessions, worktree and branch {branch}.{note}"
            )),
            |text, warning| format!("{text} {warning}"),
        ))
    }
}
