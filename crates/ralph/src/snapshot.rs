use ralph_core::snapshot::{
    commit_tree_args, delete_payload, delete_refs_args, gc_args, identity_env, index_env,
    index_path, init_args, list_prefix_args, next_turn, parse_turns, refs_to_prune, side_dir,
    update_ref_args, CHAT_REF_PREFIX, KEEP_TURNS,
};
use ralph_core::worker::project_slug;
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

    fn repo_git(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .envs(index_env(&self.side, &self.owner))
            .stdin(Stdio::null());
        command
    }

    pub fn write_tree(&self) -> io::Result<String> {
        self.ensure_side()?;
        stdout_of(&mut self.repo_git(&["add", "-A"]))?;
        stdout_of(&mut self.repo_git(&["write-tree"]))
    }

    pub fn snapshot(&self, ref_name: &str) -> io::Result<()> {
        let tree = self.write_tree()?;
        let commit =
            stdout_of(git(&commit_tree_args(&self.side, &tree, ref_name)).envs(identity_env()))?;
        stdout_of(&mut git(&update_ref_args(&self.side, ref_name, &commit)))?;
        Ok(())
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
