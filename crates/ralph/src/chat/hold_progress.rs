use crate::chat::bounded::stdout_within;
use crate::chat::prompt::terminal_columns;
use crate::config::AppConfig;
use crate::snapshot::{base_with, ORIGIN_HEAD_ARGS};
use crate::workers::running_workers;
use ralph_core::chat_hold::HoldFilter;
use ralph_core::chat_hold_progress::{
    held_ids, last_hint, lines, rebase_progress, redraw, HoldProgress,
};
use ralph_core::chat_status::RunningWorker;
use ralph_core::worker::{
    activity_path, branch_name, worktree_for_branch, ActivityMark, HarnessId,
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

const GATHER_EVERY: Duration = Duration::from_secs(30);
const COMMAND_LIMIT: Duration = Duration::from_secs(3);

pub(super) struct HoldPanel {
    color: bool,
    started: Instant,
    gathered: Option<Receiver<Vec<HoldProgress>>>,
    _stop: Sender<()>,
    facts: Vec<HoldProgress>,
    shown: Vec<String>,
    bar: String,
    pub(super) painted: usize,
}

impl HoldPanel {
    pub(super) fn new(enabled: bool, filter: &HoldFilter, color: bool, store_dir: &Path) -> Self {
        let (stop, stopped) = mpsc::channel();
        let gathered = enabled.then(|| {
            let (sender, receiver) = mpsc::channel();
            let (store_dir, filter) = (store_dir.to_path_buf(), filter.clone());
            thread::spawn(move || gather_loop(&store_dir, &filter, &sender, &stopped));
            receiver
        });
        Self {
            color,
            started: Instant::now(),
            gathered,
            _stop: stop,
            facts: Vec::new(),
            shown: Vec::new(),
            bar: String::new(),
            painted: 0,
        }
    }

    pub(super) fn tick(&mut self) {
        if let Some(gathered) = &self.gathered {
            if let Some(latest) = gathered.try_iter().last() {
                self.facts = latest;
            }
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
    stdout_within(command.args(args), COMMAND_LIMIT)
}

fn pane_of(session: &str) -> Option<String> {
    let mut command = Command::new("tmux");
    command.args(["capture-pane", "-p", "-t", &format!("={session}:")]);
    stdout_within(&mut command, COMMAND_LIMIT)
}

fn gather_loop(
    store_dir: &Path,
    filter: &HoldFilter,
    sender: &Sender<Vec<HoldProgress>>,
    stopped: &Receiver<()>,
) {
    let config = AppConfig::load().map_err(|e| e.to_string());
    loop {
        if sender.send(gather(store_dir, filter, &config)).is_err() {
            return;
        }
        if !matches!(
            stopped.recv_timeout(GATHER_EVERY),
            Err(RecvTimeoutError::Timeout)
        ) {
            return;
        }
    }
}

fn gather(
    store_dir: &Path,
    filter: &HoldFilter,
    config: &Result<AppConfig, String>,
) -> Vec<HoldProgress> {
    let running = running_workers(store_dir);
    let ids: Vec<String> = running.iter().map(|worker| worker.id.clone()).collect();
    let held = held_ids(filter, &ids);
    let porcelain = git_out(None, &["worktree", "list", "--porcelain"]);
    let base = base_with(config, git_out(None, &ORIGIN_HEAD_ARGS).as_deref());
    running
        .iter()
        .filter(|worker| held.contains(&worker.id))
        .map(|worker| {
            let tree = porcelain
                .as_deref()
                .and_then(|listing| worktree_for_branch(listing, &branch_name(&worker.id)));
            progress_of(store_dir, worker, tree.as_deref(), &base)
        })
        .collect()
}

fn progress_of(
    store_dir: &Path,
    worker: &RunningWorker,
    tree: Option<&Path>,
    base: &str,
) -> HoldProgress {
    HoldProgress {
        id: worker.id.clone(),
        elapsed_secs: 0,
        activity: std::fs::read_to_string(activity_path(store_dir, &worker.id))
            .ok()
            .and_then(|text| ActivityMark::parse(&text))
            .map(|mark| mark.state),
        rebase: tree.and_then(rebase_of),
        commits: tree.and_then(|tree| commits_since(tree, base)),
        hint: (worker.kind == HarnessId::Claude)
            .then(|| pane_of(&worker.session))
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
