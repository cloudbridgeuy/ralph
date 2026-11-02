use crate::chat::monitor::Monitor;
use crate::chat::prompt::terminal_columns;
use crate::config::AppConfig;
use crate::snapshot::{base_with, stdout_of, ORIGIN_HEAD_ARGS};
use crate::workers::{capture_pane, running_workers};
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_hold_progress::{
    due, held_ids, last_hint, lines, rebase_progress, redraw, HoldProgress,
};
use ralph_core::chat_status::RunningWorker;
use ralph_core::worker::{
    activity_path, branch_name, worktree_for_branch, ActivityMark, HarnessId,
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const GATHER_EVERY_SECS: u64 = 30;

pub(super) struct HoldPanel {
    enabled: bool,
    filter: HoldFilter,
    color: bool,
    started: Instant,
    gathered_at: Option<u64>,
    facts: Vec<HoldProgress>,
    shown: Vec<String>,
    bar: String,
    pub(super) painted: usize,
}

impl HoldPanel {
    pub(super) fn new(enabled: bool, filter: &HoldFilter, color: bool) -> Self {
        Self {
            enabled,
            filter: filter.clone(),
            color,
            started: Instant::now(),
            gathered_at: None,
            facts: Vec::new(),
            shown: Vec::new(),
            bar: String::new(),
            painted: 0,
        }
    }

    pub(super) fn tick(&mut self, monitor: &Monitor) {
        let now = self.started.elapsed().as_secs();
        if self.enabled && due(self.gathered_at, now, GATHER_EVERY_SECS) {
            self.facts = gather(monitor, &self.filter);
            self.gathered_at = Some(now);
        }
    }

    fn rendered(&self, width: usize) -> Vec<String> {
        let elapsed_secs = self.started.elapsed().as_secs();
        let facts: Vec<HoldProgress> = self
            .facts
            .iter()
            .map(|fact| HoldProgress {
                elapsed_secs,
                ..fact.clone()
            })
            .collect();
        lines(&facts, width, self.color)
    }

    pub(super) fn stale(&self, width: usize) -> bool {
        self.rendered(width) != self.shown
    }

    pub(super) fn paint(&mut self, bar: &str, width: usize) {
        let rendered = self.rendered(width);
        self.write(&rendered, bar);
        self.shown = rendered;
        self.bar = bar.to_string();
        self.painted = width;
    }

    pub(super) fn clear(&mut self) {
        if self.shown.is_empty() {
            return;
        }
        let bar = self.bar.clone();
        self.write(&[], &bar);
        self.shown.clear();
    }

    fn write(&self, rendered: &[String], bar: &str) {
        let mut out = io::stdout();
        let frame = redraw(
            rendered,
            self.shown.len(),
            bar,
            self.painted,
            terminal_columns(),
        );
        let _ = write!(out, "{frame}");
        let _ = out.flush();
    }
}

fn git_out(dir: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("--no-optional-locks");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    stdout_of(command.args(args).stdin(Stdio::null())).ok()
}

fn gather(monitor: &Monitor, filter: &HoldFilter) -> Vec<HoldProgress> {
    let running = running_workers(&monitor.store_dir);
    let ids: Vec<String> = running.iter().map(|worker| worker.id.clone()).collect();
    let held = held_ids(filter, &ids);
    let porcelain = git_out(None, &["worktree", "list", "--porcelain"]);
    let config = AppConfig::load().map_err(|e| e.to_string());
    let base = base_with(&config, git_out(None, &ORIGIN_HEAD_ARGS).as_deref());
    running
        .iter()
        .filter(|worker| held.contains(&worker.id))
        .map(|worker| {
            let tree = porcelain
                .as_deref()
                .and_then(|listing| worktree_for_branch(listing, &branch_name(&worker.id)));
            progress_of(monitor, worker, tree.as_deref(), &base)
        })
        .collect()
}

fn progress_of(
    monitor: &Monitor,
    worker: &RunningWorker,
    tree: Option<&Path>,
    base: &str,
) -> HoldProgress {
    HoldProgress {
        id: worker.id.clone(),
        elapsed_secs: 0,
        activity: std::fs::read_to_string(activity_path(&monitor.store_dir, &worker.id))
            .ok()
            .and_then(|text| ActivityMark::parse(&text))
            .map(|mark| mark.state),
        rebase: tree.and_then(rebase_of),
        commits: tree.and_then(|tree| commits_since(tree, base)),
        hint: (worker.kind == HarnessId::Claude)
            .then(|| capture_pane(&worker.session))
            .flatten()
            .and_then(|pane| last_hint(&pane)),
    }
}

fn rebase_of(tree: &Path) -> Option<(u32, u32)> {
    let dir = PathBuf::from(git_out(
        Some(tree),
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "rebase-merge",
        ],
    )?);
    let msgnum = std::fs::read_to_string(dir.join("msgnum")).ok()?;
    let end = std::fs::read_to_string(dir.join("end")).ok()?;
    rebase_progress(&msgnum, &end)
}

fn commits_since(tree: &Path, base: &str) -> Option<u32> {
    git_out(
        Some(tree),
        &["rev-list", "--count", &format!("{base}..HEAD")],
    )?
    .parse()
    .ok()
}
