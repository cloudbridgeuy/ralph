use ralph_core::cursor_leftovers::{
    project_dirs, session_cwd, session_names_worktree, slug_owner_conflict,
};
use ralph_core::worker::worktree_paths;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn remove_dir(path: &Path, messages: &mut Vec<String>) {
    match fs::remove_dir_all(path) {
        Ok(()) => messages.push(format!("Removed {}.", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => messages.push(format!(
            "Could not remove the Cursor folder {}: {e}.",
            path.display()
        )),
    }
}

fn session_dirs(sessions: &Path) -> Vec<PathBuf> {
    fs::read_dir(sessions)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

fn with_canonical(path: String) -> Vec<String> {
    let canonical = fs::canonicalize(&path)
        .ok()
        .map(|real| real.to_string_lossy().into_owned());
    std::iter::once(path).chain(canonical).collect()
}

pub fn cleanup_cursor_leftovers(
    home: &Path,
    worktree: &Path,
    real: &Path,
    porcelain: Option<&str>,
) -> Vec<String> {
    let cursor = home.join(".cursor");
    let worktree = worktree.to_string_lossy();
    let real = real.to_string_lossy();
    let mut messages = Vec::new();
    let sessions: Vec<(PathBuf, String)> = session_dirs(&cursor.join("acp-sessions"))
        .into_iter()
        .filter_map(|dir| {
            let meta = fs::read_to_string(dir.join("meta.json")).ok()?;
            Some((dir, meta))
        })
        .collect();
    match porcelain {
        None => messages.push(format!(
            "Kept the Cursor project folders of {real}: cannot prove ownership, the git worktree list failed."
        )),
        Some(porcelain) => {
            let others: Vec<String> = worktree_paths(porcelain)
                .into_iter()
                .chain(sessions.iter().filter_map(|(_, meta)| session_cwd(meta)))
                .flat_map(with_canonical)
                .collect();
            match slug_owner_conflict(&worktree, &real, &others) {
                Some(other) => messages.push(format!(
                    "Kept the Cursor project folders of {real}: {other} maps to the same folder."
                )),
                None => {
                    for dir in project_dirs(&cursor.join("projects"), &real) {
                        remove_dir(&dir, &mut messages);
                    }
                }
            }
        }
    }
    for (dir, meta) in &sessions {
        if session_names_worktree(meta, &worktree, &real) {
            remove_dir(dir, &mut messages);
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(home: &Path, id: &str, meta: &str) -> PathBuf {
        let dir = home.join(".cursor/acp-sessions").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("meta.json"), meta).unwrap();
        dir
    }

    #[test]
    fn removes_project_dir_and_matching_sessions_only() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join(".cursor/projects/p-wt");
        let other_project = home.path().join(".cursor/projects/p-wt2");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&other_project).unwrap();
        let mine = session(home.path(), "s1", "{\"cwd\":\"/w/wt\"}");
        let canonical = session(home.path(), "s2", "{\"cwd\":\"/p/wt\"}");
        let other = session(home.path(), "s3", "{\"cwd\":\"/w/wt2\"}");
        let broken = session(home.path(), "s4", "{nope");
        let messages = cleanup_cursor_leftovers(
            home.path(),
            Path::new("/w/wt"),
            Path::new("/p/wt"),
            Some(""),
        );
        assert_eq!(messages.len(), 3);
        assert!(messages.contains(&format!("Removed {}.", project.display())));
        assert!(!project.exists() && !mine.exists() && !canonical.exists());
        assert!(other.exists() && broken.exists() && other_project.exists());
    }

    #[test]
    fn missing_cursor_dir_is_silent() {
        let home = tempfile::tempdir().unwrap();
        assert!(
            cleanup_cursor_leftovers(home.path(), Path::new("/w"), Path::new("/w"), Some(""))
                .is_empty()
        );
    }

    #[test]
    fn shared_slug_keeps_project_dir_but_removes_sessions() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join(".cursor/projects/x-a-b");
        fs::create_dir_all(&project).unwrap();
        let mine = session(home.path(), "s1", "{\"cwd\":\"/x/a.b\"}");
        session(home.path(), "s2", "{\"cwd\":\"/x/a-b\"}");
        let messages = cleanup_cursor_leftovers(
            home.path(),
            Path::new("/x/a.b"),
            Path::new("/x/a.b"),
            Some(""),
        );
        assert!(project.exists() && !mine.exists());
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().any(|m| m.contains("/x/a-b")));
    }

    #[test]
    fn failed_listing_keeps_project_dir() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join(".cursor/projects/w");
        fs::create_dir_all(&project).unwrap();
        let messages =
            cleanup_cursor_leftovers(home.path(), Path::new("/w"), Path::new("/w"), None);
        assert!(project.exists());
        assert!(messages[0].contains("cannot prove ownership"));
    }
}
