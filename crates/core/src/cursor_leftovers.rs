use crate::worker::{CloseState, CloseStep};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SOCKET_BASE_MAX: usize = 84;
const SOCKET_PATH_MAX: usize = 92;
const SOCKET_HASH_CHARS: usize = 7;

#[derive(Deserialize)]
struct SessionMeta {
    cwd: String,
}

pub fn cleans_cursor_leftovers(state: &CloseState, steps: &[CloseStep]) -> bool {
    state.cursor
        && steps
            .iter()
            .any(|step| matches!(step, CloseStep::RemoveWorktree { .. }))
}

pub fn cursor_slug(path: &str) -> String {
    let mangled: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    mangled
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn socket_dir(base: &str, slug: &str) -> Option<String> {
    if base.len() > SOCKET_BASE_MAX {
        return None;
    }
    let full = format!("{base}/{slug}");
    if full.len() <= SOCKET_PATH_MAX {
        return Some(full);
    }
    let digest = Sha256::digest(full.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!(
        "{}-{}",
        full.get(..SOCKET_BASE_MAX)?,
        &hex[..SOCKET_HASH_CHARS]
    ))
}

pub fn project_dirs(projects_base: &Path, real_worktree: &str) -> Vec<PathBuf> {
    let slug = cursor_slug(real_worktree);
    let mut dirs = vec![projects_base.join(&slug)];
    let socket = projects_base
        .to_str()
        .and_then(|base| socket_dir(base, &slug))
        .map(PathBuf::from);
    if let Some(socket) = socket.filter(|socket| !dirs.contains(socket)) {
        dirs.push(socket);
    }
    dirs
}

pub fn session_names_worktree(meta_json: &str, worktree: &str, real: &str) -> bool {
    serde_json::from_str::<SessionMeta>(meta_json)
        .is_ok_and(|meta| meta.cwd == worktree || meta.cwd == real)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::Owner;

    fn state(cursor: bool) -> CloseState {
        CloseState {
            id: "A".into(),
            owner: Owner::Missing,
            dirty_worktree: Some(false),
            base: "main".into(),
            unmerged_commits: Some(0),
            opencode: false,
            markers: false,
            cursor,
        }
    }

    #[test]
    fn cursor_worker_removing_its_worktree_is_cleaned() {
        let steps = [CloseStep::RemoveWorktree { force: false }];
        assert!(cleans_cursor_leftovers(&state(true), &steps));
        assert!(!cleans_cursor_leftovers(&state(false), &steps));
        assert!(!cleans_cursor_leftovers(
            &state(true),
            &[CloseStep::ClearMarkers]
        ));
    }

    #[test]
    fn slug_collapses_and_trims() {
        assert_eq!(
            cursor_slug("/private/tmp/guz570probe/wt/a-b.c d"),
            "private-tmp-guz570probe-wt-a-b-c-d"
        );
        assert_eq!(
            cursor_slug("/Users/guzmanmonne/.claude"),
            "Users-guzmanmonne-claude"
        );
    }

    fn dirs_for(real: &str) -> Vec<String> {
        project_dirs(Path::new("/Users/guzmanmonne/.cursor/projects"), real)
            .iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn short_path_has_one_dir() {
        assert_eq!(
            dirs_for("/private/tmp/x/wt"),
            ["/Users/guzmanmonne/.cursor/projects/private-tmp-x-wt"]
        );
    }

    #[test]
    fn long_paths_get_hashed_socket_dir() {
        let cases = [
            (
                "/Users/guzmanmonne/Projects/Rust/ralph/.worktrees/cursor-smoke",
                "/Users/guzmanmonne/.cursor/projects/Users-guzmanmonne-Projects-Rust-ralph-worktrees--11db232",
            ),
            (
                "/Users/guzmanmonne/Projects/Rust/ralph/.worktrees/cursor-smoke2",
                "/Users/guzmanmonne/.cursor/projects/Users-guzmanmonne-Projects-Rust-ralph-worktrees--994d430",
            ),
            (
                "/private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpHVgYgu",
                "/Users/guzmanmonne/.cursor/projects/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr0-f404dd6",
            ),
        ];
        for (real, socket) in cases {
            let dirs = dirs_for(real);
            assert_eq!(dirs.len(), 2);
            assert_eq!(dirs[1], socket);
        }
    }

    #[test]
    fn long_projects_base_skips_socket_dir() {
        let base = format!("/{}/.cursor/projects", "h".repeat(80));
        let dirs = project_dirs(
            Path::new(&base),
            "/Users/guzmanmonne/Projects/Rust/ralph/.worktrees/cursor-smoke",
        );
        assert_eq!(dirs.len(), 1);
    }

    #[test]
    fn session_match_is_exact() {
        let meta = |cwd: &str| format!("{{\"schemaVersion\":1,\"cwd\":\"{cwd}\",\"title\":\"t\"}}");
        assert!(session_names_worktree(&meta("/a/wt"), "/a/wt", "/p/a/wt"));
        assert!(session_names_worktree(&meta("/p/a/wt"), "/a/wt", "/p/a/wt"));
        assert!(!session_names_worktree(&meta("/a/wt2"), "/a/wt", "/p/a/wt"));
        assert!(!session_names_worktree(
            &meta("/a/wt/sub"),
            "/a/wt",
            "/p/a/wt"
        ));
    }

    #[test]
    fn unparseable_meta_never_matches() {
        assert!(!session_names_worktree("{nope", "/a/wt", "/a/wt"));
        assert!(!session_names_worktree("{}", "/a/wt", "/a/wt"));
    }
}
