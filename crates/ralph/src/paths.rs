use std::path::PathBuf;

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
