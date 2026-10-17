use crate::config::AppConfig;
use crate::snapshot::{stdout_of, Snapshotter};
use crate::warn::warn_if_err;
use ralph_core::changes::{
    append_changes, diff_args, render_changes, spawn_request, turn_request, ChangeRequest,
    SnapshotRange,
};
use ralph_core::snapshot::{
    next_worker_turn, parse_worker_turns, worker_owner, worker_ref, worker_ref_prefix, worker_refs,
    worker_refs_to_prune, WorkerSite, KEEP_TURNS,
};
use ralph_core::worker::base_branch;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct WorkerChanges {
    snapshotter: Snapshotter,
    id: String,
    worktree: PathBuf,
}

fn run_diff(args: &[String], configure: impl FnOnce(&mut Command)) -> io::Result<String> {
    let (program, rest) = args
        .split_first()
        .ok_or_else(|| io::Error::other("empty diff command"))?;
    let mut command = Command::new(program);
    command.args(rest).stdin(Stdio::null());
    configure(&mut command);
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

fn git_line(worktree: &Path, args: &[&str]) -> Option<String> {
    stdout_of(
        Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(args)
            .stdin(Stdio::null()),
    )
    .ok()
    .filter(|line| !line.is_empty())
}

fn base_of(worktree: &Path) -> String {
    let config = AppConfig::load().map_err(|e| e.to_string());
    base_branch(
        config
            .as_ref()
            .map(|config| config.chat.worker_base.as_deref())
            .map_err(String::as_str),
        git_line(
            worktree,
            &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"],
        )
        .as_deref(),
    )
    .branch
}

impl WorkerChanges {
    pub fn open(site: &WorkerSite) -> Option<Self> {
        Some(Self {
            snapshotter: Snapshotter::open_worktree(
                &site.store,
                &site.worktree,
                &worker_owner(&site.id),
            )?,
            id: site.id.clone(),
            worktree: site.worktree.clone(),
        })
    }

    fn turns(&self) -> io::Result<Vec<u64>> {
        Ok(parse_worker_turns(
            &self.id,
            &self.snapshotter.refs(&worker_ref_prefix(&self.id))?,
        ))
    }

    pub fn snapshot_turn(&self) -> io::Result<()> {
        let turn = next_worker_turn(&self.turns()?);
        self.snapshotter.snapshot(&worker_ref(&self.id, turn))?;
        self.snapshotter
            .delete_refs(&worker_refs_to_prune(&self.id, &self.turns()?, KEEP_TURNS))
    }

    pub fn snapshot_turn_or_warn(&self) {
        warn_if_err(self.snapshot_turn(), "Failed to snapshot the worker turn");
    }

    pub fn clear(&self) -> io::Result<()> {
        self.snapshotter
            .delete_refs(&worker_refs(&self.id, &self.turns()?))?;
        self.snapshotter.remove_index()
    }

    fn since_spawn(&self, tree: &str) -> io::Result<String> {
        let merge_base = stdout_of(
            Command::new("git")
                .arg("-C")
                .arg(&self.worktree)
                .args(["merge-base", "HEAD", &base_of(&self.worktree)])
                .stdin(Stdio::null()),
        )?;
        let request = spawn_request(&self.id);
        let range = SnapshotRange {
            from: merge_base,
            to: Some(tree.to_string()),
        };
        let output = run_diff(&diff_args(&request, &range), |command| {
            command
                .env(
                    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                    self.snapshotter.side().join("objects"),
                )
                .current_dir(&self.worktree);
        })?;
        Ok(render_changes(&request, &output))
    }

    fn since_last_turn(&self, tree: String) -> io::Result<String> {
        let latest = self
            .turns()?
            .into_iter()
            .max()
            .ok_or_else(|| io::Error::other("no turn snapshot yet"))?;
        let request = turn_request(&self.id, latest + 1);
        self.render_in_side(
            &request,
            &SnapshotRange {
                from: worker_ref(&self.id, latest),
                to: Some(tree),
            },
        )
    }

    fn render_in_side(&self, request: &ChangeRequest, range: &SnapshotRange) -> io::Result<String> {
        let output = run_diff(&diff_args(request, range), |command| {
            command
                .env("GIT_DIR", self.snapshotter.side())
                .current_dir(self.snapshotter.side());
        })?;
        Ok(render_changes(request, &output))
    }

    pub fn attach_to(&self, text: &str) -> String {
        let blocks: Vec<String> = self
            .snapshotter
            .write_tree()
            .into_iter()
            .flat_map(|tree| [self.since_spawn(&tree), self.since_last_turn(tree)])
            .filter_map(Result::ok)
            .collect();
        append_changes(text, &blocks)
    }
}
