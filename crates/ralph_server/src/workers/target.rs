use super::{git_value, repo_root, run, Workers};
use ralph_core::worker::{
    decode_spawn_record, parse_worktree_entries, project_slug, resolve_target, spawn_record_path,
    SpawnRecord, SpawnTarget, TargetFacts, TargetRequest,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn resolve_spawn_target(request: &TargetRequest) -> io::Result<SpawnTarget> {
    let cwd = fs::canonicalize(std::env::current_dir()?)?;
    let repo_real = real_absolute(request.repo);
    let lane_real = real_absolute(request.lane);
    let request = &TargetRequest {
        repo: repo_real.as_deref().or(request.repo),
        lane: lane_real.as_deref().or(request.lane),
    };
    let TargetRequest { repo, lane } = *request;
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

fn real_absolute(path: Option<&str>) -> Option<String> {
    let path = path?.trim();
    Path::new(path)
        .is_absolute()
        .then(|| fs::canonicalize(path).ok())
        .flatten()
        .map(|real| real.to_string_lossy().into_owned())
}

impl Workers {
    pub(crate) fn spawn_record(&self, id: &str) -> Option<SpawnRecord> {
        fs::read_to_string(spawn_record_path(&self.store_dir, id))
            .ok()
            .and_then(|raw| decode_spawn_record(&raw))
    }
}

pub(crate) fn locate_repo(record: Option<&SpawnRecord>) -> io::Result<(PathBuf, String)> {
    match record.and_then(SpawnRecord::recorded_repo) {
        Some(found) => Ok(found),
        None => {
            let repo = repo_root()?;
            let project = project_slug(&repo);
            Ok((repo, project))
        }
    }
}
