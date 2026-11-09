use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn chat_resume_summary_defaults_on_and_a_store_file_can_turn_it_off() {
    assert!(AppConfig::new().chat.resume_summary);
    let temp_dir = TempDir::new().unwrap();
    let global = temp_dir.path().join("global.toml");
    let store = temp_dir.path().join("store.toml");
    fs::write(&global, "[chat]\nresume_summary = true\n").unwrap();
    fs::write(&store, "[chat]\nresume_summary = false\n").unwrap();
    assert!(
        AppConfig::load_layers(&global, None)
            .unwrap()
            .chat
            .resume_summary
    );
    assert!(
        !AppConfig::load_layers(&global, Some(&store))
            .unwrap()
            .chat
            .resume_summary
    );
}
