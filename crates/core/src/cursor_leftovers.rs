use crate::worker::{CloseState, CloseStep, HarnessId};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const SOCKET_BASE_MAX: usize = 83;
const SOCKET_PATH_MAX: usize = 91;
const SOCKET_HASH_CHARS: usize = 7;

#[derive(Deserialize)]
struct SessionMeta {
    cwd: String,
}

pub fn cleans_cursor_leftovers(state: &CloseState, steps: &[CloseStep]) -> bool {
    matches!(state.kind, HarnessId::Cursor)
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

pub fn socket_project_dir(projects_base: &Path, real_worktree: &str) -> Option<PathBuf> {
    projects_base
        .to_str()
        .and_then(|base| socket_dir(base, &cursor_slug(real_worktree)))
        .map(PathBuf::from)
}

pub fn project_dirs(projects_base: &Path, real_worktree: &str) -> Vec<PathBuf> {
    let mut dirs = vec![projects_base.join(cursor_slug(real_worktree))];
    let socket = socket_project_dir(projects_base, real_worktree);
    if let Some(socket) = socket.filter(|socket| !dirs.contains(socket)) {
        dirs.push(socket);
    }
    dirs
}

pub fn session_cwd(meta_json: &str) -> Option<String> {
    serde_json::from_str::<SessionMeta>(meta_json)
        .ok()
        .map(|meta| meta.cwd)
}

pub fn session_names_worktree(meta_json: &str, worktree: &str, real: &str) -> bool {
    session_cwd(meta_json).is_some_and(|cwd| cwd == worktree || cwd == real)
}

pub fn slug_owner_conflict<'a>(
    worktree: &str,
    real: &str,
    others: &'a [String],
) -> Option<&'a String> {
    let slug = cursor_slug(real);
    others
        .iter()
        .find(|other| *other != worktree && *other != real && cursor_slug(other) == slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::Owner;

    fn state(kind: HarnessId) -> CloseState {
        CloseState {
            id: "A".into(),
            owner: Owner::Missing,
            dirty_worktree: Some(false),
            base: "main".into(),
            unmerged_commits: Some(0),
            opencode: false,
            markers: false,
            kind,
        }
    }

    #[test]
    fn cursor_worker_removing_its_worktree_is_cleaned() {
        let steps = [CloseStep::RemoveWorktree { force: false }];
        assert!(cleans_cursor_leftovers(&state(HarnessId::Cursor), &steps));
        assert!(!cleans_cursor_leftovers(
            &state(HarnessId::Opencode),
            &steps
        ));
        assert!(!cleans_cursor_leftovers(&state(HarnessId::Claude), &steps));
        assert!(!cleans_cursor_leftovers(
            &state(HarnessId::Cursor),
            &[CloseStep::ClearMarkers]
        ));
    }

    #[test]
    fn opencode_close_skips_cursor_and_claude_leftovers() {
        let state = state(HarnessId::Opencode);
        let steps = crate::worker::close_plan(&state, false, false);
        assert!(steps.is_ok_and(|steps| {
            !cleans_cursor_leftovers(&state, &steps)
                && !crate::claude_leftovers::cleans_claude_leftovers(&state, &steps)
        }));
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
                "/Users/guzmanmonne/.cursor/projects/Users-guzmanmonne-Projects-Rust-ralph-worktrees-11db232",
            ),
            (
                "/Users/guzmanmonne/Projects/Rust/ralph/.worktrees/cursor-smoke2",
                "/Users/guzmanmonne/.cursor/projects/Users-guzmanmonne-Projects-Rust-ralph-worktrees-994d430",
            ),
            (
                "/private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpHVgYgu",
                "/Users/guzmanmonne/.cursor/projects/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr-f404dd6",
            ),
            (
                "/private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpX4HBM2",
                "/Users/guzmanmonne/.cursor/projects/private-var-folders-g9-4b8tv1yj0tnfsp21spqtx5zr-bc092f5",
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

    fn conflict(worktree: &str, real: &str, others: &[&str]) -> Option<String> {
        let others: Vec<String> = others.iter().map(|other| other.to_string()).collect();
        slug_owner_conflict(worktree, real, &others).cloned()
    }

    #[test]
    fn lossy_slug_collision_is_a_conflict() {
        assert_eq!(
            conflict("/x/a.b", "/x/a.b", &["/x/a-b"]),
            Some("/x/a-b".to_string())
        );
    }

    #[test]
    fn same_path_or_other_slug_is_no_conflict() {
        assert_eq!(conflict("/x/a.b", "/x/a.b", &["/x/a.b", "/x/c"]), None);
    }

    #[test]
    fn non_canonical_spelling_of_the_worktree_is_no_conflict() {
        assert_eq!(
            conflict("/tmp/x/wt", "/private/tmp/x/wt", &["/tmp/x/wt"]),
            None
        );
    }

    #[test]
    fn unparsable_meta_never_matches() {
        assert!(!session_names_worktree("{nope", "/a/wt", "/a/wt"));
        assert!(!session_names_worktree("{}", "/a/wt", "/a/wt"));
    }
}
