use crate::snapshot::{base_of, run_diff, stdout_of, Snapshotter};
use crate::warn::warn_if_err;
use ralph_core::changes::{
    append_changes, diff_args, render_changes, spawn_request, turn_request, SnapshotRange,
};
use ralph_core::snapshot::{
    next_worker_turn, parse_worker_turns, worker_owner, worker_ref, worker_ref_prefix, worker_refs,
    worker_refs_to_prune, WorkerSite, KEEP_TURNS,
};
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub struct WorkerChanges {
    snapshotter: Snapshotter,
    id: String,
    worktree: PathBuf,
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

    pub fn snapshot_turn(&self) -> io::Result<u64> {
        let turn = next_worker_turn(&self.turns()?);
        self.snapshotter.snapshot(&worker_ref(&self.id, turn))?;
        self.snapshotter.delete_refs(&worker_refs_to_prune(
            &self.id,
            &self.turns()?,
            KEEP_TURNS,
        ))?;
        Ok(turn)
    }

    pub fn snapshot_turn_or_warn(&self) -> Option<u64> {
        let turn = self.snapshot_turn();
        let snapshotted = turn.as_ref().ok().copied();
        warn_if_err(turn.map(|_| ()), "Failed to snapshot the worker turn");
        snapshotted
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
        self.snapshotter.render_diff(
            &request,
            &SnapshotRange {
                from: worker_ref(&self.id, latest),
                to: Some(tree),
            },
            self.snapshotter.current_base(),
        )
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
