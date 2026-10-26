use super::{check, git_value};
use ralph_core::worker::{guard_config_steps, hook_scripts, HOOKS_DIR, UNSET_ALL_FLAG};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const KEY_ABSENT_EXIT: i32 = 5;

pub(super) fn apply(repo: &Path, worktree: &Path, store_dir: &Path) -> io::Result<()> {
    let hooks_dir = store_dir.join(HOOKS_DIR);
    write_hooks(&hooks_dir, &common_hooks_dir(repo)?)?;
    for step in guard_config_steps(&hooks_dir) {
        let output = Command::new("git")
            .arg("-C")
            .arg(worktree)
            .args(&step)
            .stdin(Stdio::null())
            .output()?;
        let key_absent = step.iter().any(|arg| arg == UNSET_ALL_FLAG)
            && output.status.code() == Some(KEY_ABSENT_EXIT);
        if !key_absent {
            check(output)?;
        }
    }
    Ok(())
}

fn common_hooks_dir(repo: &Path) -> io::Result<PathBuf> {
    let common = git_value(repo, &["rev-parse", "--git-common-dir"])?
        .ok_or_else(|| io::Error::other("could not find the git common directory"))?;
    Ok(repo.join(common).join("hooks"))
}

fn hook_names(common_hooks: &Path) -> io::Result<Vec<String>> {
    if !common_hooks.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(common_hooks)? {
        let entry = entry?;
        if is_executable_file(&entry.path()) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn write_hooks(hooks_dir: &Path, common_hooks: &Path) -> io::Result<()> {
    fs::create_dir_all(hooks_dir)?;
    for script in hook_scripts(common_hooks, &hook_names(common_hooks)?) {
        let tmp = hooks_dir.join(format!(".{}.{}.tmp", script.name, std::process::id()));
        fs::write(&tmp, &script.content)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
        fs::rename(&tmp, hooks_dir.join(&script.name))?;
    }
    Ok(())
}
