use crate::chat::thread::config_root;
use crate::config::AppConfig;
use crate::warn::warn_once;
use ralph_core::changes::{
    diff_args, diff_base, merge_tree_args, merged_tree, render_changes, with_note, ChangeRequest,
    DiffBase, SnapshotRange, UPSTREAM_NOTE,
};
use ralph_core::snapshot::{
    append_lexically, commit_tree_args, delete_payload, delete_refs_args, exclude_base, gc_args,
    identity_env, index_env, index_lock_path, index_path, index_steps, init_args, list_prefix_args,
    next_turn, not_excludable_message, parse_snapshot_base, parse_turns, refs_to_prune, side_dir,
    snapshot_excludes, snapshot_lock_expired, snapshot_lock_path, snapshot_message,
    update_ref_args, IndexStep, SnapshotBase, CHAT_REF_PREFIX, HEAD_CHECK_ARGS, KEEP_TURNS,
    SNAPSHOT_LOCK_POLL,
};
use ralph_core::worker::{base_branch, project_slug};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

pub struct Snapshotter {
    repo: PathBuf,
    side: PathBuf,
    owner: String,
    excludes: Vec<String>,
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

pub(crate) const ORIGIN_HEAD_ARGS: [&str; 3] = ["symbolic-ref", "-q", "refs/remotes/origin/HEAD"];

pub(crate) fn base_with(config: &Result<AppConfig, String>, origin_head: Option<&str>) -> String {
    base_branch(
        config
            .as_ref()
            .map(|config| config.chat.worker_base.as_deref())
            .map_err(String::as_str),
        origin_head,
    )
    .branch
}

pub(crate) fn base_of(worktree: &Path) -> String {
    let config = AppConfig::load().map_err(|e| e.to_string());
    base_with(&config, git_line(worktree, &ORIGIN_HEAD_ARGS).as_deref())
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

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn repo_of(cwd: &Path) -> Option<PathBuf> {
    rev_parse(cwd, "--show-toplevel")
}

fn main_repo_of(cwd: &Path) -> Option<PathBuf> {
    rev_parse(cwd, "--git-common-dir")?
        .parent()
        .map(Path::to_path_buf)
}

fn canonical_or_planned(path: &Path, base: &Path) -> PathBuf {
    let full = base.join(path);
    if let Ok(resolved) = fs::canonicalize(&full) {
        return resolved;
    }
    full.ancestors()
        .find_map(|ancestor| {
            let resolved = fs::canonicalize(ancestor).ok()?;
            let tail = full.strip_prefix(ancestor).ok()?;
            Some(append_lexically(&resolved, tail))
        })
        .unwrap_or(full)
}

fn excludes_for_store(repo: &Path, store: &Path, side: &Path, slug: &str) -> Vec<String> {
    match exclude_base(store, std::env::current_dir().ok()) {
        Some(base) => excludes_for(repo, store, side, slug, &base),
        None => {
            warn_once(
                "snapshot-store-relative-no-cwd",
                format!(
                    "the ralph store {} is relative and the working directory is unavailable; snapshots will not exclude it",
                    store.display()
                ),
            );
            Vec::new()
        }
    }
}

fn excludes_for(repo: &Path, store: &Path, side: &Path, slug: &str, base: &Path) -> Vec<String> {
    let repo = canonical_or_planned(repo, base);
    let written = base.join(store);
    let mapped = config_root(&written);
    let main_store = if mapped == written {
        config_root(&canonical_or_planned(store, base))
    } else {
        mapped
    };
    let store = canonical_or_planned(&main_store, base);
    let main_side = canonical_or_planned(&side_dir(&main_store, slug), base);
    if store == repo {
        warn_once(
            "snapshot-store-is-repo-root",
            format!(
                "the ralph store {} is the repo root; snapshots skip only its snapshots folders",
                store.display()
            ),
        );
    }
    let side = canonical_or_planned(side, base);
    if let Some(message) = not_excludable_message(&repo, &store, &side) {
        warn_once("snapshot-store-not-excludable", message);
    }
    snapshot_excludes(&repo, &store, &side, &main_side)
}

impl Snapshotter {
    pub fn open(store: &Path, cwd: &Path, owner: &str) -> Option<Self> {
        let repo = repo_of(cwd)?;
        let slug = project_slug(&repo);
        let side = side_dir(store, &slug);
        let excludes = excludes_for_store(&repo, store, &side, &slug);
        Some(Self {
            repo,
            side,
            owner: owner.to_string(),
            excludes,
        })
    }

    pub fn open_worktree(store: &Path, worktree: &Path, owner: &str) -> Option<Self> {
        let repo = repo_of(worktree)?;
        let slug = project_slug(&main_repo_of(worktree)?);
        let side = side_dir(store, &slug);
        let excludes = excludes_for_store(&repo, store, &side, &slug);
        Some(Self {
            repo,
            side,
            owner: owner.to_string(),
            excludes,
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

    fn repo_command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .stdin(Stdio::null());
        command
    }

    fn repo_git<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = self.repo_command(args);
        command.envs(index_env(&self.side, &self.owner));
        command
    }

    // HEAD's objects live in the repo, not the side dir, so read-tree swaps
    // only the index. Stat-clean entries keep blobs an earlier add wrote to
    // the side dir; the rest are rehashed by add -A.
    fn step_git(&self, step: IndexStep) -> Command {
        match step {
            IndexStep::ReadEmpty | IndexStep::ReadHead => {
                let mut command = self.repo_command(step.args(&[]));
                command.env("GIT_INDEX_FILE", index_path(&self.side, &self.owner));
                command
            }
            IndexStep::AddAll => self.repo_git(step.args(&self.excludes)),
        }
    }

    fn lock_index(&self) -> io::Result<fs::File> {
        let path = snapshot_lock_path(&self.side, &self.owner);
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(fs::TryLockError::WouldBlock) => {}
                Err(fs::TryLockError::Error(error)) => return Err(error),
            }
            if snapshot_lock_expired(started.elapsed()) {
                return Err(io::Error::other(format!(
                    "skipped: snapshot lock {} is still held",
                    path.display()
                )));
            }
            thread::sleep(SNAPSHOT_LOCK_POLL);
        }
    }

    pub fn write_tree(&self) -> io::Result<String> {
        self.ensure_side()?;
        let _lock = self.lock_index()?;
        remove_if_present(&index_lock_path(&self.side, &self.owner))?;
        let has_head = self
            .repo_command(HEAD_CHECK_ARGS)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success();
        for step in index_steps(has_head) {
            stdout_of(&mut self.step_git(step))?;
        }
        stdout_of(&mut self.repo_git(["write-tree"]))
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

    pub(crate) fn side_env(&self, command: &mut Command) {
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

    /// Rebase-aware range for a diff, plus `UPSTREAM_NOTE` when the base is unknown.
    pub fn adjust_range(
        &self,
        range: &SnapshotRange,
        to_base: Option<SnapshotBase>,
    ) -> (SnapshotRange, Option<&'static str>) {
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
        (
            SnapshotRange {
                from,
                to: range.to.clone(),
            },
            note,
        )
    }

    pub fn render_diff(
        &self,
        request: &ChangeRequest,
        range: &SnapshotRange,
        to_base: Option<SnapshotBase>,
    ) -> io::Result<String> {
        let (range, note) = self.adjust_range(range, to_base);
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
        let _lock = match self.lock_index() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            lock => lock?,
        };
        remove_if_present(&index_path(&self.side, &self.owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::snapshot::{snapshot_lock_path, worker_owner};

    fn snapshotter_in(side: &Path) -> Snapshotter {
        Snapshotter {
            repo: side.to_path_buf(),
            side: side.to_path_buf(),
            owner: worker_owner("A"),
            excludes: Vec::new(),
        }
    }

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn add_all_carries_the_excludes_and_read_head_does_not() {
        let mut snapshotter = snapshotter_in(Path::new("/s"));
        snapshotter.excludes = vec![":(exclude,literal,top).ralph".to_string()];
        let add = args_of(&snapshotter.step_git(IndexStep::AddAll));
        assert_eq!(
            add[add.len() - 5..],
            ["add", "-A", "--", ".", ":(exclude,literal,top).ralph"]
        );
        let read = args_of(&snapshotter.step_git(IndexStep::ReadHead));
        assert_eq!(read[read.len() - 4..], ["read-tree", "-m", "-i", "HEAD"]);
        let empty = args_of(&snapshotter.step_git(IndexStep::ReadEmpty));
        assert_eq!(empty[empty.len() - 2..], ["read-tree", "--empty"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_thread_store_still_excludes_the_main_store() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let repo = root.join("repo");
        let main = repo.join(".ralph");
        let outside = root.join("outside");
        fs::create_dir_all(main.join("threads")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let thread_store = main.join("threads").join("t");
        std::os::unix::fs::symlink(&outside, &thread_store).unwrap();
        let side = side_dir(&thread_store, "slug");
        let excludes = excludes_for(&repo, &thread_store, &side, "slug", Path::new("/"));
        assert!(excludes.contains(&":(exclude,literal,top).ralph".to_string()));
    }

    #[test]
    fn a_missing_tail_is_re_appended_to_the_canonical_existing_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let planned = canonical_or_planned(&dir.path().join("a").join("b"), Path::new("/"));
        assert_eq!(planned, root.join("a").join("b"));
    }

    #[cfg(unix)]
    #[test]
    fn a_dot_dotted_store_resolves_symlinks_before_the_parent_part() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let sub = root.join("repo").join("sub");
        fs::create_dir_all(&sub).unwrap();
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(&sub, home.join("link")).unwrap();
        let store = home.join("link").join("..").join(".ralph");
        let repo = root.join("repo");
        let side = root.join("elsewhere");
        let excludes = excludes_for(&repo, &store, &side, "slug", Path::new("/"));
        assert_eq!(excludes, [":(exclude,literal,top).ralph"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_kept_as_written_under_a_resolved_parent() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), real.join("link")).unwrap();
        std::os::unix::fs::symlink(&real, root.join("alias")).unwrap();
        let resolved = canonical_or_planned(&Path::new("alias").join("link"), &root);
        assert_eq!(resolved, real.join("link"));
    }

    #[test]
    fn a_store_inside_the_repo_is_excluded_even_before_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let store = repo.join(".ralph");
        let side = side_dir(&store, "slug");
        assert_eq!(
            excludes_for(&repo, &store, &side, "slug", dir.path()),
            [":(exclude,literal,top).ralph"]
        );
    }

    #[test]
    fn a_thread_store_excludes_the_main_store_not_only_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let store = repo.join(".ralph").join("threads").join("foo");
        let side = side_dir(&store, "slug");
        assert_eq!(
            excludes_for(&repo, &store, &side, "slug", dir.path()),
            [":(exclude,literal,top).ralph"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_store_resolves_to_its_target_inside_the_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(repo.join("real")).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(repo.join("real"), &link).unwrap();
        let side = side_dir(&link, "slug");
        assert_eq!(
            excludes_for(&repo, &link, &side, "slug", dir.path()),
            [":(exclude,literal,top)real"]
        );
    }

    #[test]
    fn a_dot_dotted_thread_store_excludes_the_main_store() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let store = repo
            .join(".ralph")
            .join("threads")
            .join("x")
            .join("..")
            .join("t");
        let side = side_dir(&store, "slug");
        assert_eq!(
            excludes_for(&repo, &store, &side, "slug", dir.path()),
            [":(exclude,literal,top).ralph"]
        );
    }

    #[test]
    fn a_thread_store_under_a_repo_root_main_store_excludes_both_side_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let store = repo.join("threads").join("t");
        let side = side_dir(&store, "slug");
        let excludes = excludes_for(&repo, &store, &side, "slug", dir.path());
        let thread_side = side.strip_prefix(&repo).unwrap().display().to_string();
        let main_side = side_dir(&repo, "slug")
            .strip_prefix(&repo)
            .unwrap()
            .display()
            .to_string();
        assert!(excludes.contains(&format!(":(exclude,literal,top){thread_side}")));
        assert!(excludes.contains(&format!(":(exclude,literal,top){main_side}")));
        assert!(excludes
            .iter()
            .all(|e| e != ":(exclude,literal,top)" && e != ":(exclude,literal,top)."));
    }

    #[test]
    fn a_relative_store_resolves_against_the_base() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let side = side_dir(Path::new(".ralph"), "slug");
        assert_eq!(
            excludes_for(&repo, Path::new(".ralph"), &side, "slug", &repo),
            [":(exclude,literal,top).ralph"]
        );
    }

    #[test]
    fn a_missing_store_with_parent_parts_is_excluded_by_its_clean_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        let store = repo.join("x").join("..").join("y").join(".ralph");
        let side = side_dir(&store, "slug");
        assert_eq!(
            excludes_for(&repo, &store, &side, "slug", dir.path()),
            [":(exclude,literal,top)y/.ralph"]
        );
    }

    #[test]
    fn remove_index_deletes_the_index_but_keeps_its_snapshot_lock() {
        let dir = tempfile::tempdir().unwrap();
        let snapshotter = snapshotter_in(dir.path());
        let index = index_path(dir.path(), &snapshotter.owner);
        fs::create_dir_all(index.parent().unwrap()).unwrap();
        fs::write(&index, "").unwrap();
        fs::write(snapshot_lock_path(dir.path(), &snapshotter.owner), "").unwrap();
        snapshotter.remove_index().unwrap();
        assert!(!index.exists());
        assert!(snapshot_lock_path(dir.path(), &snapshotter.owner).exists());
    }

    #[test]
    fn remove_index_without_a_side_dir_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        assert!(snapshotter_in(&dir.path().join("missing"))
            .remove_index()
            .is_ok());
    }
}
