use ralph_core::cursor_leftovers::{project_dirs, session_names_worktree};
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

fn matching_sessions(sessions: &Path, worktree: &str, real: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(sessions) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| {
            fs::read_to_string(dir.join("meta.json"))
                .is_ok_and(|meta| session_names_worktree(&meta, worktree, real))
        })
        .collect()
}

pub fn cleanup_cursor_leftovers(home: &Path, worktree: &Path, real: &Path) -> Vec<String> {
    let cursor = home.join(".cursor");
    let worktree = worktree.to_string_lossy();
    let real = real.to_string_lossy();
    let mut messages = Vec::new();
    for dir in project_dirs(&cursor.join("projects"), &real) {
        remove_dir(&dir, &mut messages);
    }
    for dir in matching_sessions(&cursor.join("acp-sessions"), &worktree, &real) {
        remove_dir(&dir, &mut messages);
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
        let messages =
            cleanup_cursor_leftovers(home.path(), Path::new("/w/wt"), Path::new("/p/wt"));
        assert_eq!(messages.len(), 3);
        assert!(messages.contains(&format!("Removed {}.", project.display())));
        assert!(!project.exists() && !mine.exists() && !canonical.exists());
        assert!(other.exists() && broken.exists() && other_project.exists());
    }

    #[test]
    fn missing_cursor_dir_is_silent() {
        let home = tempfile::tempdir().unwrap();
        assert!(cleanup_cursor_leftovers(home.path(), Path::new("/w"), Path::new("/w")).is_empty());
    }
}
