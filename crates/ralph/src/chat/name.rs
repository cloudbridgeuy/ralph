use ralph_core::chat_name::{git_root_candidates, resolve_name, NameSources};
use std::path::{Path, PathBuf};

pub(super) fn resolved_home() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.canonicalize().unwrap_or(home))
}

pub(super) fn session_name(
    flag: Option<&str>,
    config: Option<&str>,
    cwd: &Path,
    home: Option<&Path>,
) -> String {
    resolve_name(&NameSources {
        flag,
        config,
        git_root: git_root_candidates(cwd, home)
            .into_iter()
            .find(|dir| dir.join(".git").exists()),
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
        assert_eq!(session_name(None, None, &deep, None), "my-repo");
    }

    #[test]
    fn git_file_counts_as_a_repo_root() {
        let root = TempDir::new().unwrap();
        let tree = root.path().join("wt-GUZ-9");
        let deep = tree.join("crates");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(session_name(None, None, &deep, None), "wt-GUZ-9");
    }

    #[test]
    fn flag_and_config_beat_the_repo_name() {
        let root = TempDir::new().unwrap();
        let repo = root.path().join("my-repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(session_name(Some("f"), Some("c"), &repo, None), "f");
        assert_eq!(session_name(None, Some("c"), &repo, None), "c");
    }

    #[test]
    fn git_directory_in_home_does_not_name_the_chat() {
        let home = TempDir::new().unwrap();
        let work = home.path().join("scratch/inner");
        std::fs::create_dir_all(home.path().join(".git")).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        assert_eq!(session_name(None, None, &work, Some(home.path())), "inner");
    }

    #[test]
    fn git_directory_below_home_still_names_the_chat() {
        let home = TempDir::new().unwrap();
        let repo = home.path().join("proj");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        assert_eq!(
            session_name(None, None, &repo.join("src"), Some(home.path())),
            "proj"
        );
    }
}
