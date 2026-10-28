use ralph_core::claude_leftovers::{
    claude_dir, claude_json, remove_project_entry, transcript_dir_name, trust_swap_step,
    trust_temp_name, TrustSwap,
};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

fn fingerprint(path: &Path) -> io::Result<Fingerprint> {
    let meta = fs::metadata(path)?;
    Ok(Fingerprint {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

fn ignore_missing(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn remove_transcript_dir(claude_dir: &Path, worktree: &Path) -> io::Result<()> {
    let dir = claude_dir
        .join("projects")
        .join(transcript_dir_name(&worktree.to_string_lossy()));
    ignore_missing(fs::remove_dir_all(dir))
}

enum TrustOutcome {
    Removed,
    Unchanged,
    Raced,
    NoLock,
}

fn remove_trust_entry(
    path: &Path,
    lock: Option<&Path>,
    worktree: &Path,
) -> io::Result<TrustOutcome> {
    if !path.try_exists()? {
        return Ok(TrustOutcome::Unchanged);
    }
    let Some(lock) = lock else {
        return Ok(TrustOutcome::NoLock);
    };
    if let Some(dir) = lock.parent() {
        fs::create_dir_all(dir)?;
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock)?;
    lock.lock()?;
    let temp = path.with_file_name(trust_temp_name(
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed),
    ));
    let mut attempt = 0;
    loop {
        let before = match fingerprint(path) {
            Ok(before) => before,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(TrustOutcome::Unchanged),
            Err(e) => return Err(e),
        };
        let text = fs::read_to_string(path)?;
        let Some(updated) = remove_project_entry(&text, &worktree.to_string_lossy())
            .map_err(|e| io::Error::other(format!("cannot parse {}: {e}", path.display())))?
        else {
            return Ok(TrustOutcome::Unchanged);
        };
        fs::write(&temp, updated)?;
        let unchanged = fingerprint(path).ok().as_ref() == Some(&before);
        match trust_swap_step(unchanged, attempt) {
            TrustSwap::Commit => {
                fs::rename(&temp, path).inspect_err(|_| {
                    let _ = fs::remove_file(&temp);
                })?;
                return Ok(TrustOutcome::Removed);
            }
            TrustSwap::Retry => attempt += 1,
            TrustSwap::GiveUp => {
                let _ = fs::remove_file(&temp);
                return Ok(TrustOutcome::Raced);
            }
        }
    }
}

pub fn cleanup_worktree_leftovers(
    config_dir: Option<&Path>,
    home: &Path,
    lock: Option<&Path>,
    worktree: &Path,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(e) = remove_transcript_dir(&claude_dir(config_dir, home), worktree) {
        warnings.push(format!(
            "Could not remove the Claude transcript folder for {}: {e}.",
            worktree.display()
        ));
    }
    match remove_trust_entry(&claude_json(config_dir, home), lock, worktree) {
        Ok(TrustOutcome::Removed | TrustOutcome::Unchanged) => {}
        Ok(TrustOutcome::Raced) => warnings.push(format!(
            "Skipped removing the trust entry for {} from ~/.claude.json: the file changed while ralph worked.",
            worktree.display()
        )),
        Ok(TrustOutcome::NoLock) => warnings.push(format!(
            "Skipped removing the trust entry for {} from ~/.claude.json: no per-user data folder for ralph's lock.",
            worktree.display()
        )),
        Err(e) => warnings.push(format!(
            "Could not remove the trust entry for {} from ~/.claude.json: {e}.",
            worktree.display()
        )),
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with(config: Option<&str>) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        if let Some(config) = config {
            fs::write(home.path().join(".claude.json"), config).unwrap();
        }
        home
    }

    #[test]
    fn removes_transcript_dir_and_trust_entry() {
        let home = home_with(Some("{\"projects\":{\"/w/a\":{},\"/w/b\":{}}}"));
        let dir = home.path().join(".claude/projects/-w-a");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("t.jsonl"), "x").unwrap();
        let warnings = cleanup_worktree_leftovers(
            None,
            home.path(),
            Some(&home.path().join("data/ralph/lock")),
            Path::new("/w/a"),
        );
        assert!(warnings.is_empty());
        assert!(!dir.exists());
        let config = fs::read_to_string(home.path().join(".claude.json")).unwrap();
        assert_eq!(config, "{\n  \"projects\": {\n    \"/w/b\": {}\n  }\n}");
        let leftovers: Vec<_> = fs::read_dir(home.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn config_dir_moves_the_transcript_dir_and_trust_file() {
        let home = home_with(Some("{\"projects\":{\"/w/a\":{}}}"));
        let alt = home.path().join("alt");
        fs::create_dir_all(alt.join("projects/-w-a")).unwrap();
        fs::write(
            alt.join(".claude.json"),
            "{\"projects\":{\"/w/a\":{},\"/w/b\":{}}}",
        )
        .unwrap();
        let lock = home.path().join("data/ralph/lock");
        let warnings =
            cleanup_worktree_leftovers(Some(&alt), home.path(), Some(&lock), Path::new("/w/a"));
        assert!(warnings.is_empty());
        assert!(!alt.join("projects/-w-a").exists());
        let moved = fs::read_to_string(alt.join(".claude.json")).unwrap();
        assert_eq!(moved, "{\n  \"projects\": {\n    \"/w/b\": {}\n  }\n}");
        let untouched = fs::read_to_string(home.path().join(".claude.json")).unwrap();
        assert_eq!(untouched, "{\"projects\":{\"/w/a\":{}}}");
    }

    #[test]
    fn missing_folder_and_file_are_silent() {
        let home = home_with(None);
        let lock = home.path().join("data/ralph/lock");
        assert!(
            cleanup_worktree_leftovers(None, home.path(), Some(&lock), Path::new("/w/a"))
                .is_empty()
        );
        assert!(!lock.exists());
    }

    #[test]
    fn missing_lock_dir_warns_and_keeps_file() {
        let text = "{\"projects\":{\"/w/a\":{}}}";
        let home = home_with(Some(text));
        let warnings = cleanup_worktree_leftovers(None, home.path(), None, Path::new("/w/a"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            text
        );
    }

    #[test]
    fn missing_key_leaves_file_untouched() {
        let text = "{\"projects\":{\"/w/b\":{}}}";
        let home = home_with(Some(text));
        assert!(cleanup_worktree_leftovers(
            None,
            home.path(),
            Some(&home.path().join("data/ralph/lock")),
            Path::new("/w/a")
        )
        .is_empty());
        assert_eq!(
            fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            text
        );
    }

    #[test]
    fn invalid_json_warns_and_keeps_file() {
        let home = home_with(Some("{nope"));
        let warnings = cleanup_worktree_leftovers(
            None,
            home.path(),
            Some(&home.path().join("data/ralph/lock")),
            Path::new("/w/a"),
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            "{nope"
        );
    }
}
