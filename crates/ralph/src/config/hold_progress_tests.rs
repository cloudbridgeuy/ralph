use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn chat_hold_progress_defaults_on_and_can_be_disabled() {
    assert!(AppConfig::new().chat.hold_progress);
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    fs::write(&path, "[chat]\nhold_progress = false\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(!config.chat.hold_progress);
    fs::write(&path, "[chat]\nedit_mode = \"vi\"\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    assert!(config.chat.hold_progress);
}
