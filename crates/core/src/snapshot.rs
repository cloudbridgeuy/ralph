use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const KEEP_TURNS: usize = 20;
pub const CHAT_OWNER: &str = "chat";
pub const CHAT_REF_PREFIX: &str = "refs/ralph/chat/";

pub fn snapshots_enabled(config_enabled: Option<bool>) -> bool {
    config_enabled.unwrap_or(true)
}

const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "ralph"),
    ("GIT_AUTHOR_EMAIL", "ralph@localhost"),
    ("GIT_COMMITTER_NAME", "ralph"),
    ("GIT_COMMITTER_EMAIL", "ralph@localhost"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

impl Edge {
    fn name(self) -> &'static str {
        match self {
            Edge::Start => "start",
            Edge::End => "end",
        }
    }
}

pub fn side_dir(store: &Path, slug: &str) -> PathBuf {
    store.join("snapshots").join(format!("{slug}.git"))
}

pub fn index_path(side: &Path, owner: &str) -> PathBuf {
    side.join("index").join(owner)
}

pub fn chat_ref(turn: u64, edge: Edge) -> String {
    format!("{CHAT_REF_PREFIX}turn-{turn}/{}", edge.name())
}

pub fn parse_turns(ref_list: &str) -> Vec<u64> {
    ref_list
        .lines()
        .filter_map(|line| line.trim().strip_prefix(CHAT_REF_PREFIX))
        .filter_map(|rest| rest.strip_prefix("turn-"))
        .filter_map(|rest| rest.split_once('/'))
        .filter(|(_, edge)| *edge == "start" || *edge == "end")
        .filter_map(|(turn, _)| turn.parse().ok())
        .collect::<BTreeSet<u64>>()
        .into_iter()
        .collect()
}

pub fn next_turn(turns: &[u64]) -> u64 {
    turns.iter().max().map_or(1, |max| max + 1)
}

pub fn refs_to_prune(turns: &[u64], keep: usize) -> Vec<String> {
    let mut sorted = turns.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let pruned = sorted.len().saturating_sub(keep);
    sorted
        .into_iter()
        .take(pruned)
        .flat_map(|turn| [chat_ref(turn, Edge::Start), chat_ref(turn, Edge::End)])
        .collect()
}

pub fn delete_payload(refs: &[String]) -> String {
    refs.iter().map(|name| format!("delete {name}\n")).collect()
}

pub fn index_env(side: &Path, owner: &str) -> [(&'static str, OsString); 2] {
    [
        (
            "GIT_OBJECT_DIRECTORY",
            side.join("objects").into_os_string(),
        ),
        ("GIT_INDEX_FILE", index_path(side, owner).into_os_string()),
    ]
}

pub fn identity_env() -> [(&'static str, &'static str); 4] {
    IDENTITY
}

pub fn commit_tree_args(side: &Path, tree: &str, message: &str) -> Vec<OsString> {
    vec![
        "-c".into(),
        "commit.gpgsign=false".into(),
        "--git-dir".into(),
        side.as_os_str().to_owned(),
        "commit-tree".into(),
        tree.into(),
        "-m".into(),
        message.into(),
    ]
}

pub fn update_ref_args(side: &Path, name: &str, commit: &str) -> Vec<OsString> {
    vec![
        "--git-dir".into(),
        side.as_os_str().to_owned(),
        "update-ref".into(),
        name.into(),
        commit.into(),
    ]
}

pub fn list_refs_args(side: &Path) -> Vec<OsString> {
    vec![
        "--git-dir".into(),
        side.as_os_str().to_owned(),
        "for-each-ref".into(),
        "--format=%(refname)".into(),
        CHAT_REF_PREFIX.into(),
    ]
}

pub fn delete_refs_args(side: &Path) -> Vec<OsString> {
    vec![
        "--git-dir".into(),
        side.as_os_str().to_owned(),
        "update-ref".into(),
        "--stdin".into(),
    ]
}

pub fn gc_args(side: &Path) -> Vec<OsString> {
    vec![
        "--git-dir".into(),
        side.as_os_str().to_owned(),
        "gc".into(),
        "--auto".into(),
        "--prune=1.hour.ago".into(),
    ]
}

pub fn init_args(side: &Path) -> Vec<OsString> {
    vec![
        "init".into(),
        "--bare".into(),
        "-q".into(),
        side.as_os_str().to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_follow_the_loaded_config() {
        assert!(snapshots_enabled(Some(true)));
        assert!(!snapshots_enabled(Some(false)));
    }

    #[test]
    fn snapshots_default_on_when_the_config_failed_to_load() {
        assert!(snapshots_enabled(None));
    }

    #[test]
    fn side_and_index_paths() {
        let side = side_dir(Path::new("/s"), "ralph");
        assert_eq!(side, Path::new("/s/snapshots/ralph.git"));
        assert_eq!(
            index_path(&side, "chat"),
            Path::new("/s/snapshots/ralph.git/index/chat")
        );
    }

    #[test]
    fn chat_refs_name_turn_and_edge() {
        assert_eq!(chat_ref(3, Edge::Start), "refs/ralph/chat/turn-3/start");
        assert_eq!(chat_ref(12, Edge::End), "refs/ralph/chat/turn-12/end");
    }

    #[test]
    fn parse_turns_dedups_sorts_and_skips_noise() {
        let list = "refs/ralph/chat/turn-10/end\nrefs/ralph/chat/turn-2/start\nrefs/ralph/chat/turn-2/end\nrefs/ralph/chat/turn-x/end\nrefs/ralph/chat/turn-4/mid\nrefs/ralph/workers/a/turn-9/end\n\n";
        assert_eq!(parse_turns(list), [2, 10]);
        assert!(parse_turns("").is_empty());
    }

    #[test]
    fn next_turn_continues_after_max() {
        assert_eq!(next_turn(&[]), 1);
        assert_eq!(next_turn(&[2, 7, 5]), 8);
    }

    #[test]
    fn refs_to_prune_drops_oldest_turns_with_both_edges() {
        assert_eq!(
            refs_to_prune(&[5, 1, 3, 2], 2),
            [
                "refs/ralph/chat/turn-1/start",
                "refs/ralph/chat/turn-1/end",
                "refs/ralph/chat/turn-2/start",
                "refs/ralph/chat/turn-2/end",
            ]
        );
    }

    #[test]
    fn refs_to_prune_is_empty_within_keep() {
        assert!(refs_to_prune(&[1, 2], 2).is_empty());
        assert!(refs_to_prune(&[], 20).is_empty());
        assert!(refs_to_prune(&[1], 0).len() == 2);
    }

    #[test]
    fn delete_payload_has_one_line_per_ref() {
        let refs = vec!["refs/a".to_string(), "refs/b".to_string()];
        assert_eq!(delete_payload(&refs), "delete refs/a\ndelete refs/b\n");
        assert_eq!(delete_payload(&[]), "");
    }

    #[test]
    fn index_env_points_at_side_objects_and_owner_index() {
        let env = index_env(Path::new("/s/x.git"), "chat");
        assert_eq!(env[0].0, "GIT_OBJECT_DIRECTORY");
        assert_eq!(env[0].1, OsString::from("/s/x.git/objects"));
        assert_eq!(env[1].0, "GIT_INDEX_FILE");
        assert_eq!(env[1].1, OsString::from("/s/x.git/index/chat"));
    }

    #[test]
    fn identity_env_sets_author_and_committer() {
        let names: Vec<&str> = identity_env().iter().map(|(k, _)| *k).collect();
        assert_eq!(
            names,
            [
                "GIT_AUTHOR_NAME",
                "GIT_AUTHOR_EMAIL",
                "GIT_COMMITTER_NAME",
                "GIT_COMMITTER_EMAIL"
            ]
        );
    }

    fn strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn commit_tree_args_disable_signing() {
        assert_eq!(
            strings(commit_tree_args(Path::new("/s.git"), "abc", "turn 1")),
            [
                "-c",
                "commit.gpgsign=false",
                "--git-dir",
                "/s.git",
                "commit-tree",
                "abc",
                "-m",
                "turn 1"
            ]
        );
    }

    #[test]
    fn git_dir_argv_builders() {
        let side = Path::new("/s.git");
        assert_eq!(
            strings(update_ref_args(side, "refs/x", "c1")),
            ["--git-dir", "/s.git", "update-ref", "refs/x", "c1"]
        );
        assert_eq!(
            strings(list_refs_args(side)),
            [
                "--git-dir",
                "/s.git",
                "for-each-ref",
                "--format=%(refname)",
                "refs/ralph/chat/"
            ]
        );
        assert_eq!(
            strings(delete_refs_args(side)),
            ["--git-dir", "/s.git", "update-ref", "--stdin"]
        );
        assert_eq!(
            strings(gc_args(side)),
            ["--git-dir", "/s.git", "gc", "--auto", "--prune=1.hour.ago"]
        );
        assert_eq!(strings(init_args(side)), ["init", "--bare", "-q", "/s.git"]);
    }
}
