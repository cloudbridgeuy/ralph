use crate::worker::{CloseState, CloseStep, WorkerKind};
use serde_json::Value;

const PROJECTS_KEY: &str = "projects";

pub fn cleans_claude_leftovers(state: &CloseState, steps: &[CloseStep]) -> bool {
    matches!(state.kind, WorkerKind::Tmux)
        && steps
            .iter()
            .any(|step| matches!(step, CloseStep::RemoveWorktree { .. }))
}

pub fn transcript_dir_name(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn remove_project_entry(
    config: &str,
    worktree_path: &str,
) -> Result<Option<String>, serde_json::Error> {
    let mut root: Value = serde_json::from_str(config)?;
    let removed = root
        .get_mut(PROJECTS_KEY)
        .and_then(Value::as_object_mut)
        .and_then(|projects| projects.shift_remove(worktree_path));
    if removed.is_none() {
        return Ok(None);
    }
    let mut text = serde_json::to_string_pretty(&root)?;
    if config.ends_with('\n') {
        text.push('\n');
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::Owner;

    fn state(kind: WorkerKind) -> CloseState {
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
    fn claude_worker_removing_its_worktree_is_cleaned() {
        let steps = [CloseStep::RemoveWorktree { force: false }];
        assert!(cleans_claude_leftovers(&state(WorkerKind::Tmux), &steps));
    }

    #[test]
    fn cursor_worker_is_not_cleaned() {
        let steps = [CloseStep::RemoveWorktree { force: false }];
        assert!(!cleans_claude_leftovers(&state(WorkerKind::Cursor), &steps));
        assert!(!cleans_claude_leftovers(
            &state(WorkerKind::Opencode),
            &steps
        ));
    }

    #[test]
    fn close_without_worktree_removal_is_not_cleaned() {
        let steps = [CloseStep::DeleteBranch, CloseStep::ClearMarkers];
        assert!(!cleans_claude_leftovers(&state(WorkerKind::Tmux), &steps));
    }

    #[test]
    fn transcript_dir_name_mangles_slashes_and_dots() {
        assert_eq!(
            transcript_dir_name("/Users/x/repo/.worktrees/GUZ-1"),
            "-Users-x-repo--worktrees-GUZ-1"
        );
    }

    #[test]
    fn transcript_dir_name_mangles_underscore_and_space() {
        assert_eq!(transcript_dir_name("/a_b c/d"), "-a-b-c-d");
        assert_eq!(
            transcript_dir_name("/private/var/folders/c_/g_m011"),
            "-private-var-folders-c--g-m011"
        );
    }

    #[test]
    fn remove_project_entry_drops_key_and_keeps_order() {
        let config = "{\n  \"a\": 1,\n  \"projects\": {\n    \"/z\": {\n      \"t\": true\n    },\n    \"/w\": {},\n    \"/b\": {}\n  },\n  \"c\": 2\n}";
        let out = remove_project_entry(config, "/w").unwrap().unwrap();
        assert_eq!(
            out,
            "{\n  \"a\": 1,\n  \"projects\": {\n    \"/z\": {\n      \"t\": true\n    },\n    \"/b\": {}\n  },\n  \"c\": 2\n}"
        );
    }

    #[test]
    fn remove_project_entry_keeps_trailing_newline() {
        let out = remove_project_entry("{\"projects\":{\"/w\":{}}}\n", "/w")
            .unwrap()
            .unwrap();
        assert_eq!(out, "{\n  \"projects\": {}\n}\n");
    }

    #[test]
    fn remove_project_entry_missing_key_is_none() {
        assert_eq!(
            remove_project_entry("{\"projects\":{\"/a\":{}}}", "/w").unwrap(),
            None
        );
    }

    #[test]
    fn remove_project_entry_missing_projects_is_none() {
        assert_eq!(remove_project_entry("{\"a\":1}", "/w").unwrap(), None);
        assert_eq!(
            remove_project_entry("{\"projects\":[1]}", "/w").unwrap(),
            None
        );
    }

    #[test]
    fn remove_project_entry_invalid_json_is_error() {
        assert!(remove_project_entry("{nope", "/w").is_err());
    }
}
