use crate::worker::{CloseState, CloseStep, HarnessId};
use serde_json::Value;
use std::path::{Path, PathBuf};

const PROJECTS_KEY: &str = "projects";
const MAX_DIR_NAME_LEN: usize = 200;
const TRUST_SWAP_ATTEMPTS: usize = 3;
const TRUST_LOCK_NAME: &str = "claude-json.lock";

pub fn cleans_claude_leftovers(state: &CloseState, steps: &[CloseStep]) -> bool {
    matches!(state.kind, HarnessId::Claude)
        && steps
            .iter()
            .any(|step| matches!(step, CloseStep::RemoveWorktree { .. }))
}

pub fn transcript_dir_name(path: &str) -> String {
    let name: String = path
        .encode_utf16()
        .map(|unit| match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    if name.len() <= MAX_DIR_NAME_LEN {
        return name;
    }
    format!(
        "{}-{}",
        &name[..MAX_DIR_NAME_LEN],
        base36(path_hash(path).unsigned_abs())
    )
}

fn path_hash(path: &str) -> i32 {
    path.encode_utf16().fold(0, |hash: i32, unit| {
        hash.wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(i32::from(unit))
    })
}

fn base36(mut n: u32) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit(n % 36, 36).unwrap_or('0'));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digits.iter().rev().collect()
}

pub fn trust_lock_path(data_dir: Option<&Path>) -> Option<PathBuf> {
    data_dir.map(|dir| dir.join("ralph").join(TRUST_LOCK_NAME))
}

pub fn trust_temp_name(pid: u32, seq: u64) -> String {
    format!(".claude.json.ralph-{pid}-{seq}.tmp")
}

#[derive(Debug, PartialEq, Eq)]
pub enum TrustSwap {
    Commit,
    Retry,
    GiveUp,
}

pub fn trust_swap_step(unchanged: bool, attempt: usize) -> TrustSwap {
    if unchanged {
        TrustSwap::Commit
    } else if attempt + 1 < TRUST_SWAP_ATTEMPTS {
        TrustSwap::Retry
    } else {
        TrustSwap::GiveUp
    }
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
    fn claude_worker_removing_its_worktree_is_cleaned() {
        let steps = [CloseStep::RemoveWorktree { force: false }];
        assert!(cleans_claude_leftovers(&state(HarnessId::Claude), &steps));
    }

    #[test]
    fn cursor_worker_is_not_cleaned() {
        let steps = [CloseStep::RemoveWorktree { force: false }];
        assert!(!cleans_claude_leftovers(&state(HarnessId::Cursor), &steps));
        assert!(!cleans_claude_leftovers(
            &state(HarnessId::Opencode),
            &steps
        ));
    }

    #[test]
    fn close_without_worktree_removal_is_not_cleaned() {
        let steps = [CloseStep::DeleteBranch, CloseStep::ClearMarkers];
        assert!(!cleans_claude_leftovers(&state(HarnessId::Claude), &steps));
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
    fn transcript_dir_name_counts_utf16_units_like_claude() {
        assert_eq!(transcript_dir_name("/w/\u{1F600}\u{e9}"), "-w----");
    }

    #[test]
    fn transcript_dir_name_keeps_names_up_to_200_chars() {
        let path = format!("/{}", "b".repeat(199));
        assert_eq!(transcript_dir_name(&path), format!("-{}", "b".repeat(199)));
    }

    #[test]
    fn transcript_dir_name_truncates_and_hashes_long_paths() {
        let path = format!("/{}", "b".repeat(200));
        assert_eq!(
            transcript_dir_name(&path),
            format!("-{}-86ms41", "b".repeat(199))
        );
        let path = format!("/Users/x/{}/.worktrees/GUZ-1", "a".repeat(200));
        let name = transcript_dir_name(&path);
        assert_eq!(name.len(), 207);
        assert!(name.starts_with("-Users-x-aaa"));
        assert!(name.ends_with("a-4eg6hz"));
    }

    #[test]
    fn path_hash_matches_claude_java_style_hash() {
        assert_eq!(path_hash(&format!("/{}", "b".repeat(199))), 1646604435);
        assert_eq!(path_hash(&format!("/{}", "b".repeat(200))), -494869969);
        assert_eq!(path_hash("/w/\u{1F600}\u{e9}"), 1511829375);
    }

    #[test]
    fn base36_writes_lowercase_digits() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(35), "z");
        assert_eq!(base36(36), "10");
        assert_eq!(base36(u32::MAX), "1z141z3");
    }

    #[test]
    fn trust_lock_path_sits_in_the_user_data_dir() {
        assert_eq!(
            trust_lock_path(Some(Path::new("/u/data"))),
            Some(PathBuf::from("/u/data/ralph/claude-json.lock"))
        );
        assert_eq!(trust_lock_path(None), None);
    }

    #[test]
    fn trust_lock_path_is_the_same_for_every_store() {
        let data = Path::new("/u/data");
        let stores = [Path::new("/s/one"), Path::new("/s/two/.ralph")];
        let locks: Vec<_> = stores.iter().map(|_| trust_lock_path(Some(data))).collect();
        assert_eq!(locks[0], locks[1]);
        let lock = locks[0].clone().unwrap();
        assert!(stores.iter().all(|store| !lock.starts_with(store)));
    }

    #[test]
    fn trust_temp_name_differs_per_process_and_close() {
        assert_eq!(trust_temp_name(42, 7), ".claude.json.ralph-42-7.tmp");
        assert_ne!(trust_temp_name(42, 7), trust_temp_name(43, 7));
        assert_ne!(trust_temp_name(42, 7), trust_temp_name(42, 8));
    }

    #[test]
    fn trust_swap_commits_only_when_file_is_unchanged() {
        assert_eq!(trust_swap_step(true, 0), TrustSwap::Commit);
        assert_eq!(trust_swap_step(true, 2), TrustSwap::Commit);
    }

    #[test]
    fn trust_swap_retries_a_lost_race_then_gives_up() {
        assert_eq!(trust_swap_step(false, 0), TrustSwap::Retry);
        assert_eq!(trust_swap_step(false, 1), TrustSwap::Retry);
        assert_eq!(trust_swap_step(false, 2), TrustSwap::GiveUp);
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
    fn remove_project_entry_keeps_float_digits() {
        let config =
            "{\"cost\":1.8578832499999998,\"ms\":0.22375000000010914,\"projects\":{\"/w\":{}}}";
        let out = remove_project_entry(config, "/w").unwrap().unwrap();
        assert_eq!(
            out,
            "{\n  \"cost\": 1.8578832499999998,\n  \"ms\": 0.22375000000010914,\n  \"projects\": {}\n}"
        );
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
