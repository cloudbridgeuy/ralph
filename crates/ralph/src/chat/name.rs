use ralph_core::chat_name::{resolve_name, NameSources};
use std::path::Path;

pub(super) fn session_name(flag: Option<&str>, config: Option<&str>, cwd: &Path) -> String {
    resolve_name(&NameSources {
        flag,
        config,
        git_root: cwd.ancestors().find(|dir| dir.join(".git").exists()),
        cwd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn git_directory_at_an_ancestor_names_the_chat() {
        let root = TempDir::new().unwrap();
        let repo = root.path().join("my-repo");
        let deep = repo.join("src/inner");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(session_name(None, None, &deep), "my-repo");
    }

    #[test]
    fn git_file_counts_as_a_repo_root() {
        let root = TempDir::new().unwrap();
        let tree = root.path().join("wt-GUZ-9");
        let deep = tree.join("crates");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(session_name(None, None, &deep), "wt-GUZ-9");
    }

    #[test]
    fn flag_and_config_beat_the_repo_name() {
        let root = TempDir::new().unwrap();
        let repo = root.path().join("my-repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(session_name(Some("f"), Some("c"), &repo), "f");
        assert_eq!(session_name(None, Some("c"), &repo), "c");
    }
}
