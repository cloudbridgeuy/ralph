use super::WORKTREE_DIR;
use std::path::{Path, PathBuf};

const BRANCH_PREFIX: &str = "branch refs/heads/";
const WORKTREE_PREFIX: &str = "worktree ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeDir {
    pub path: PathBuf,
    pub warning: Option<String>,
}

pub fn resolve_worktree_dir(
    repo: &Path,
    home: Option<&Path>,
    flag: Option<&str>,
    env: Option<&str>,
    config: Result<Option<&str>, &str>,
) -> WorktreeDir {
    let (configured, config_error) = match config {
        Ok(configured) => (configured, None),
        Err(error) => (None, Some(error)),
    };
    let dir = [flag, env, configured]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|dir| !dir.is_empty())
        .unwrap_or(WORKTREE_DIR);
    let path = repo.join(expand_home(dir, home));
    let warning = config_error.map(|error| {
        format!(
            "Warning: [chat] worktree_dir was skipped because the ralph config did not load ({error}). Used {} instead.",
            path.display()
        )
    });
    WorktreeDir { path, warning }
}

fn expand_home(dir: &str, home: Option<&Path>) -> PathBuf {
    match (dir.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_path_buf(),
        (Some(rest), Some(home)) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(dir),
    }
}

pub fn worktree_for_branch(porcelain: &str, branch: &str) -> Option<PathBuf> {
    let wanted = format!("{BRANCH_PREFIX}{branch}");
    porcelain.split("\n\n").find_map(|entry| {
        let mut lines = entry.lines();
        let path = lines.next()?.strip_prefix(WORKTREE_PREFIX)?;
        lines
            .any(|line| line == wanted)
            .then(|| PathBuf::from(path))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(flag: Option<&str>, env: Option<&str>, config: Option<&str>) -> PathBuf {
        resolve_worktree_dir(Path::new("/repo"), None, flag, env, Ok(config)).path
    }

    #[test]
    fn precedence_is_flag_env_config_default() {
        assert_eq!(
            resolve(Some("f"), Some("e"), Some("c")),
            Path::new("/repo/f")
        );
        assert_eq!(resolve(None, Some("e"), Some("c")), Path::new("/repo/e"));
        assert_eq!(resolve(None, None, Some("c")), Path::new("/repo/c"));
        assert_eq!(resolve(None, None, None), Path::new("/repo/.worktrees"));
        assert_eq!(
            resolve(Some(" "), Some(""), Some("c")),
            Path::new("/repo/c")
        );
    }

    #[test]
    fn relative_paths_join_the_repo_and_absolute_paths_stay() {
        assert_eq!(resolve(None, None, Some("../wt")), Path::new("/repo/../wt"));
        assert_eq!(resolve(None, Some("/tmp/wt"), None), Path::new("/tmp/wt"));
    }

    #[test]
    fn a_leading_tilde_expands_to_home() {
        let home = Some(Path::new("/home/me"));
        let repo = Path::new("/repo");
        let path = |dir| resolve_worktree_dir(repo, home, None, Some(dir), Ok(None)).path;
        assert_eq!(path("~/wt"), Path::new("/home/me/wt"));
        assert_eq!(path("~"), Path::new("/home/me"));
        assert_eq!(path("~other/wt"), Path::new("/repo/~other/wt"));
        assert_eq!(path("a/~/wt"), Path::new("/repo/a/~/wt"));
        assert_eq!(
            resolve_worktree_dir(repo, home, None, None, Ok(Some("~/c"))).path,
            Path::new("/home/me/c")
        );
        assert_eq!(
            resolve_worktree_dir(repo, None, None, Some("~/wt"), Ok(None)).path,
            Path::new("/repo/~/wt")
        );
    }

    #[test]
    fn a_config_that_did_not_load_warns_and_falls_back() {
        let repo = Path::new("/repo");
        let failed = resolve_worktree_dir(repo, None, None, None, Err("bad toml"));
        assert_eq!(failed.path, Path::new("/repo/.worktrees"));
        let warning = failed.warning.unwrap();
        assert!(warning.contains("worktree_dir was skipped"), "{warning}");
        assert!(warning.contains("bad toml"), "{warning}");
        assert!(warning.contains("/repo/.worktrees"), "{warning}");
        let env = resolve_worktree_dir(repo, None, None, Some("e"), Err("bad"));
        assert_eq!(env.path, Path::new("/repo/e"));
        assert!(resolve_worktree_dir(repo, None, None, None, Ok(None))
            .warning
            .is_none());
    }

    #[test]
    fn worktree_for_branch_reads_git_porcelain() {
        let porcelain = "worktree /repo\nHEAD 1\nbranch refs/heads/main\n\nworktree /old/GUZ-1\nHEAD 2\nbranch refs/heads/worker/GUZ-1\nprunable gitdir file points to non-existent location\n\nworktree /d\nHEAD 3\ndetached\n";
        assert_eq!(
            worktree_for_branch(porcelain, "worker/GUZ-1"),
            Some(PathBuf::from("/old/GUZ-1"))
        );
        assert_eq!(worktree_for_branch(porcelain, "worker/GUZ"), None);
        assert_eq!(worktree_for_branch("", "worker/GUZ-1"), None);
    }
}
