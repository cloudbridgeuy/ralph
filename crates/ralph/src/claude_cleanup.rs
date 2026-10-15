use ralph_core::claude_leftovers::{remove_project_entry, transcript_dir_name};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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

fn remove_transcript_dir(home: &Path, worktree: &Path) -> io::Result<()> {
    let dir = home
        .join(".claude")
        .join("projects")
        .join(transcript_dir_name(&worktree.to_string_lossy()));
    ignore_missing(fs::remove_dir_all(dir))
}

enum TrustOutcome {
    Removed,
    Unchanged,
    Raced,
}

fn remove_trust_entry(home: &Path, worktree: &Path) -> io::Result<TrustOutcome> {
    let path = home.join(".claude.json");
    let before = match fingerprint(&path) {
        Ok(before) => before,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(TrustOutcome::Unchanged),
        Err(e) => return Err(e),
    };
    let text = fs::read_to_string(&path)?;
    let Some(updated) = remove_project_entry(&text, &worktree.to_string_lossy())
        .map_err(|e| io::Error::other(format!("cannot parse {}: {e}", path.display())))?
    else {
        return Ok(TrustOutcome::Unchanged);
    };
    let mut temp = path.clone().into_os_string();
    temp.push(".ralph-tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, updated)?;
    if fingerprint(&path).ok().as_ref() != Some(&before) {
        let _ = fs::remove_file(&temp);
        return Ok(TrustOutcome::Raced);
    }
    fs::rename(&temp, &path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })?;
    Ok(TrustOutcome::Removed)
}

pub fn cleanup_worktree_leftovers(home: &Path, worktree: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(e) = remove_transcript_dir(home, worktree) {
        warnings.push(format!(
            "Could not remove the Claude transcript folder for {}: {e}.",
            worktree.display()
        ));
    }
    match remove_trust_entry(home, worktree) {
        Ok(TrustOutcome::Removed | TrustOutcome::Unchanged) => {}
        Ok(TrustOutcome::Raced) => warnings.push(format!(
            "Skipped removing the trust entry for {} from ~/.claude.json: the file changed while ralph worked.",
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
        let warnings = cleanup_worktree_leftovers(home.path(), Path::new("/w/a"));
        assert!(warnings.is_empty());
        assert!(!dir.exists());
        let config = fs::read_to_string(home.path().join(".claude.json")).unwrap();
        assert_eq!(config, "{\n  \"projects\": {\n    \"/w/b\": {}\n  }\n}");
        assert!(!home.path().join(".claude.json.ralph-tmp").exists());
    }

    #[test]
    fn missing_folder_and_file_are_silent() {
        let home = home_with(None);
        assert!(cleanup_worktree_leftovers(home.path(), Path::new("/w/a")).is_empty());
    }

    #[test]
    fn missing_key_leaves_file_untouched() {
        let text = "{\"projects\":{\"/w/b\":{}}}";
        let home = home_with(Some(text));
        assert!(cleanup_worktree_leftovers(home.path(), Path::new("/w/a")).is_empty());
        assert_eq!(
            fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            text
        );
    }

    #[test]
    fn invalid_json_warns_and_keeps_file() {
        let home = home_with(Some("{nope"));
        let warnings = cleanup_worktree_leftovers(home.path(), Path::new("/w/a"));
        assert_eq!(warnings.len(), 1);
        assert_eq!(
            fs::read_to_string(home.path().join(".claude.json")).unwrap(),
            "{nope"
        );
    }
}
