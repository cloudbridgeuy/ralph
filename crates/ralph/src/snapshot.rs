use crate::config::AppConfig;
use ralph_core::changes::{
    diff_args, diff_base, merge_tree_args, merged_tree, render_changes, with_note, ChangeRequest,
    DiffBase, SnapshotRange, UPSTREAM_NOTE,
};
use ralph_core::snapshot::{
    commit_tree_args, delete_payload, delete_refs_args, gc_args, identity_env, index_env,
    index_path, index_steps, init_args, list_prefix_args, next_turn, parse_snapshot_base,
    parse_turns, refs_to_prune, side_dir, snapshot_message, update_ref_args, IndexStep,
    SnapshotBase, CHAT_REF_PREFIX, HEAD_CHECK_ARGS, KEEP_TURNS,
};
use ralph_core::worker::{base_branch, project_slug};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Snapshotter {
    repo: PathBuf,
    side: PathBuf,
    owner: String,
}

fn git(args: &[OsString]) -> Command {
    let mut command = Command::new("git");
    command.args(args).stdin(Stdio::null());
    command
}

pub(crate) fn stdout_of(command: &mut Command) -> io::Result<String> {
    let output = command.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

pub(crate) fn run_diff(
    args: &[String],
    configure: impl FnOnce(&mut Command),
) -> io::Result<String> {
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

pub(crate) fn git_line(worktree: &Path, args: &[&str]) -> Option<String> {
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

pub(crate) fn base_of(worktree: &Path) -> String {
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

fn rev_parse(cwd: &Path, flag: &str) -> Option<PathBuf> {
    let output = stdout_of(
        Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--path-format=absolute", flag])
            .stdin(Stdio::null()),
    )
    .ok()?;
    Some(PathBuf::from(output))
}

fn repo_of(cwd: &Path) -> Option<PathBuf> {
    rev_parse(cwd, "--show-toplevel")
}

fn main_repo_of(cwd: &Path) -> Option<PathBuf> {
    rev_parse(cwd, "--git-common-dir")?
        .parent()
        .map(Path::to_path_buf)
}

impl Snapshotter {
    pub fn open(store: &Path, cwd: &Path, owner: &str) -> Option<Self> {
        let repo = repo_of(cwd)?;
        let side = side_dir(store, &project_slug(&repo));
        Some(Self {
            repo,
            side,
            owner: owner.to_string(),
        })
    }

    pub fn open_worktree(store: &Path, worktree: &Path, owner: &str) -> Option<Self> {
        let repo = repo_of(worktree)?;
        let side = side_dir(store, &project_slug(&main_repo_of(worktree)?));
        Some(Self {
            repo,
            side,
            owner: owner.to_string(),
        })
    }

    pub fn side(&self) -> &Path {
        &self.side
    }

    fn ensure_side(&self) -> io::Result<()> {
        if !self.side.join("objects").is_dir() {
            if let Some(parent) = self.side.parent() {
                fs::create_dir_all(parent)?;
            }
            stdout_of(&mut git(&init_args(&self.side)))?;
        }
        if let Some(dir) = index_path(&self.side, &self.owner).parent() {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    fn repo_command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .stdin(Stdio::null());
        command
    }

    fn repo_git(&self, args: &[&str]) -> Command {
        let mut command = self.repo_command(args);
        command.envs(index_env(&self.side, &self.owner));
        command
    }

    // HEAD's objects live in the repo, not the side dir, so read-tree swaps
    // only the index. Stat-clean entries keep blobs an earlier add wrote to
    // the side dir; the rest are rehashed by add -A.
    fn step_git(&self, step: IndexStep) -> Command {
        match step {
            IndexStep::ReadHead => {
                let mut command = self.repo_command(step.args());
                command.env("GIT_INDEX_FILE", index_path(&self.side, &self.owner));
                command
            }
            IndexStep::AddAll => self.repo_git(step.args()),
        }
    }

    pub fn write_tree(&self) -> io::Result<String> {
        self.ensure_side()?;
        let has_head = self
            .repo_command(&HEAD_CHECK_ARGS)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success();
        for step in index_steps(has_head) {
            stdout_of(&mut self.step_git(step))?;
        }
        stdout_of(&mut self.repo_git(&["write-tree"]))
    }

    pub fn snapshot(&self, ref_name: &str) -> io::Result<()> {
        let tree = self.write_tree()?;
        let message = snapshot_message(ref_name, self.current_base().as_ref());
        let commit =
            stdout_of(git(&commit_tree_args(&self.side, &tree, &message)).envs(identity_env()))?;
        stdout_of(&mut git(&update_ref_args(&self.side, ref_name, &commit)))?;
        Ok(())
    }

    pub fn current_base(&self) -> Option<SnapshotBase> {
        let head = git_line(&self.repo, &["rev-parse", "HEAD"])?;
        let base = git_line(&self.repo, &["merge-base", "HEAD", &base_of(&self.repo)]);
        Some(SnapshotBase { head, base })
    }

    pub fn snapshot_base(&self, rev: &str) -> Option<SnapshotBase> {
        let text = stdout_of(
            Command::new("git")
                .arg("--git-dir")
                .arg(&self.side)
                .args(["cat-file", "commit", rev])
                .stdin(Stdio::null()),
        )
        .ok()?;
        parse_snapshot_base(&text)
    }

    fn side_env(&self, command: &mut Command) {
        command.env("GIT_DIR", &self.side).current_dir(&self.side);
        if let Some(common) = rev_parse(&self.repo, "--git-common-dir") {
            command.env("GIT_ALTERNATE_OBJECT_DIRECTORIES", common.join("objects"));
        }
    }

    fn rebased_onto(&self, old: &str, new: &str, snapshot: &str) -> Option<String> {
        let output = run_diff(&merge_tree_args(old, new, snapshot), |command| {
            self.side_env(command)
        })
        .ok()?;
        merged_tree(&output)
    }

    pub fn render_diff(
        &self,
        request: &ChangeRequest,
        range: &SnapshotRange,
        to_base: Option<SnapshotBase>,
    ) -> io::Result<String> {
        let from_base = self.snapshot_base(&range.from);
        let (from, note) = match diff_base(from_base.as_ref(), to_base.as_ref()) {
            DiffBase::Direct => (range.from.clone(), None),
            DiffBase::Unknown => (range.from.clone(), Some(UPSTREAM_NOTE)),
            DiffBase::Rebase { old, new } => match self.rebased_onto(&old, &new, &range.from) {
                Some(tree) => (tree, None),
                // git merge-tree --merge-base is missing (git < 2.40) or the
                // merge conflicts: diff the original snapshot and add UPSTREAM_NOTE.
                None => (range.from.clone(), Some(UPSTREAM_NOTE)),
            },
        };
        let range = SnapshotRange {
            from,
            to: range.to.clone(),
        };
        let output = run_diff(&diff_args(request, &range), |command| {
            self.side_env(command)
        })?;
        Ok(with_note(&render_changes(request, &output), note))
    }

    pub fn refs(&self, prefix: &str) -> io::Result<String> {
        if !self.side.join("objects").is_dir() {
            return Ok(String::new());
        }
        stdout_of(&mut git(&list_prefix_args(&self.side, prefix)))
    }

    pub fn turns(&self) -> io::Result<Vec<u64>> {
        Ok(parse_turns(&self.refs(CHAT_REF_PREFIX)?))
    }

    pub fn next_turn(&self) -> io::Result<u64> {
        Ok(next_turn(&self.turns()?))
    }

    pub fn prune(&self) -> io::Result<()> {
        self.delete_refs(&refs_to_prune(&self.turns()?, KEEP_TURNS))
    }

    pub fn delete_refs(&self, doomed: &[String]) -> io::Result<()> {
        if doomed.is_empty() {
            return Ok(());
        }
        let mut child = git(&delete_refs_args(&self.side))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(delete_payload(doomed).as_bytes())?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        stdout_of(&mut git(&gc_args(&self.side)))?;
        Ok(())
    }

    pub fn remove_index(&self) -> io::Result<()> {
        match fs::remove_file(index_path(&self.side, &self.owner)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
