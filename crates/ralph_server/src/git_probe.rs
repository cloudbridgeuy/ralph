use ralph_core::worker::worktree_for_branch;
use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};

pub fn branch_exists(repo: &Path, branch: &str) -> io::Result<bool> {
    Ok(Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()?
        .success())
}

fn probe_command(repo: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    command
}

fn git_output(repo: &Path, args: &[&str]) -> io::Result<Output> {
    probe_command(repo, args).output()
}

pub fn branch_open(repo: &Path, branch: &str) -> bool {
    let ref_name = format!("refs/heads/{branch}");
    let probe = git_output(repo, &["rev-parse", "--verify", "--quiet", &ref_name]);
    let listing = git_output(repo, &["worktree", "list", "--porcelain"]);
    match (probe, listing) {
        (Ok(probe), Ok(listing)) => {
            probe.status.success()
                || worktree_for_branch(&String::from_utf8_lossy(&listing.stdout), branch).is_some()
        }
        (Err(error), _) | (_, Err(error)) => {
            crate::warn::warn(format!("checking branch {branch}: {error}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn the_probe_command_removes_no_git_env_var() {
        let command = probe_command(Path::new("."), &["status"]);
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(key, value)| value.is_none() && key.to_string_lossy().starts_with("GIT_"))
            .collect();
        assert!(removed.is_empty(), "{removed:?}");
    }

    #[test]
    fn the_shared_probe_finds_a_branch_in_a_temp_repo() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["commit", "-q", "--allow-empty", "-m", "x"]);
        git(dir.path(), &["branch", "worker/sched-x"]);
        assert!(branch_exists(dir.path(), "worker/sched-x").unwrap());
        assert!(!branch_exists(dir.path(), "worker/sched-none").unwrap());
        assert!(branch_open(dir.path(), "worker/sched-x"));
    }
}
