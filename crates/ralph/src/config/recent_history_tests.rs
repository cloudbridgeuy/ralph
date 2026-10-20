use super::AppConfig;
use std::fs;
use tempfile::TempDir;

#[test]
fn chat_section_parses_recent_history_tokens() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nrecent_history_tokens = 2048\n").unwrap();
    let config = AppConfig::load_from(&path).unwrap();
    assert_eq!(config.chat.recent_history_tokens, Some(2048));
    assert_eq!(AppConfig::new().chat.recent_history_tokens, None);
}
