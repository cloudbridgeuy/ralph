use super::{git_value, repo_root, run};
use ralph_core::worker::{
    parse_worktree_entries, resolve_target, SpawnTarget, TargetFacts, TargetRequest,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn resolve_spawn_target(request: &TargetRequest) -> io::Result<SpawnTarget> {
    let TargetRequest { repo, lane } = *request;
    let cwd = fs::canonicalize(std::env::current_dir()?)?;
    let probe = repo
        .or(lane.filter(|lane| Path::new(lane.trim()).is_absolute()))
        .map(|path| cwd.join(path.trim()));
    let repo_top = probe
        .and_then(|path| fs::canonicalize(path).ok())
        .and_then(|path| {
            git_value(&path, &["rev-parse", "--show-toplevel"])
                .ok()
                .flatten()
        })
        .map(PathBuf::from);
    let entries = repo_top
        .as_deref()
        .and_then(|top| {
            run(Command::new("git")
                .arg("-C")
                .arg(top)
                .args(["worktree", "list", "--porcelain"]))
            .ok()
        })
        .map(|listing| parse_worktree_entries(&String::from_utf8_lossy(&listing.stdout)))
        .unwrap_or_default();
    let facts = TargetFacts {
        cwd_repo_top: repo_root().ok(),
        cwd,
        repo_top,
        entries,
    };
    resolve_target(request, &facts).map_err(io::Error::other)
}
