use super::{check, git_value};
use ralph_core::worker::{
    guard_config_steps, hook_scripts, hooks_dir_in, parse_remote_urls, stale_hook_names,
    HookScript, RemoteUrls, UNSET_ALL_FLAG,
};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

const KEY_ABSENT_EXIT: i32 = 5;
const NO_MATCH_EXIT: i32 = 1;
const REMOTE_URL_KEYS: &str = r"^remote\..*\.(url|pushurl)$";

static SETUP: Mutex<()> = Mutex::new(());
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn lock_setup() -> MutexGuard<'static, ()> {
    SETUP.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn apply(repo: &Path, worktree: &Path) -> io::Result<()> {
    let _setup = lock_setup();
    let common = common_dir(repo)?;
    let hooks_dir = hooks_dir_in(&common);
    let steps = guard_config_steps(&hooks_dir, &remote_urls(&common)?).map_err(io::Error::other)?;
    write_hooks(&hooks_dir, &common.join("hooks"))?;
    for step in steps {
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

fn common_dir(repo: &Path) -> io::Result<PathBuf> {
    let common = git_value(repo, &["rev-parse", "--git-common-dir"])?
        .ok_or_else(|| io::Error::other("could not find the git common directory"))?;
    Ok(repo.join(common))
}

fn remote_urls(common: &Path) -> io::Result<RemoteUrls> {
    let output = Command::new("git")
        .arg("config")
        .arg("--file")
        .arg(common.join("config"))
        .args(["--get-regexp", REMOTE_URL_KEYS])
        .stdin(Stdio::null())
        .output()?;
    if output.status.code() == Some(NO_MATCH_EXIT) {
        return Ok(RemoteUrls::default());
    }
    let output = check(output)?;
    Ok(parse_remote_urls(&String::from_utf8_lossy(&output.stdout)))
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
    let scripts = hook_scripts(common_hooks, &hook_names(common_hooks)?);
    for script in &scripts {
        write_hook(hooks_dir, script)?;
    }
    for stale in stale_hook_names(&installed_names(hooks_dir)?, &scripts) {
        fs::remove_file(hooks_dir.join(stale))?;
    }
    Ok(())
}

fn write_hook(hooks_dir: &Path, script: &HookScript) -> io::Result<()> {
    let tmp = hooks_dir.join(format!(
        ".{}.{}.{}.tmp",
        script.name,
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&tmp, &script.content)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
    fs::rename(&tmp, hooks_dir.join(&script.name))
}

fn installed_names(hooks_dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(hooks_dir)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') {
            names.push(name);
        }
    }
    Ok(names)
}
