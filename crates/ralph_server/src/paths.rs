use std::path::{Path, PathBuf};

pub const RALPH_CONFIG_DIR_ENV: &str = "RALPH_CONFIG_DIR";

pub fn config_dir() -> PathBuf {
    std::env::var(RALPH_CONFIG_DIR_ENV)
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("ralph")
        })
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn absolute_from(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_joins_onto_cwd() {
        assert_eq!(
            absolute_from(Path::new("cfg/usage.jsonl"), Path::new("/work/chat")),
            PathBuf::from("/work/chat/cfg/usage.jsonl")
        );
    }

    #[test]
    fn dot_path_joins_onto_cwd() {
        assert_eq!(
            absolute_from(Path::new("."), Path::new("/work/chat")),
            PathBuf::from("/work/chat/.")
        );
    }

    #[test]
    fn absolute_path_is_unchanged() {
        assert_eq!(
            absolute_from(Path::new("/etc/ralph/usage.jsonl"), Path::new("/work/chat")),
            PathBuf::from("/etc/ralph/usage.jsonl")
        );
    }
}
