use super::AppConfig;
use std::fs;
use tempfile::TempDir;

#[test]
fn reset_limits_default_and_warn_on_bad_values() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    let (limits, warnings) = AppConfig::new().chat.reset_limits_setting();
    assert_eq!((limits.turns, limits.tokens), (30, 140_000));
    assert!(warnings.is_empty());
    fs::write(&path, "[chat]\nreset_turns = 2\nreset_tokens = 5000\n").unwrap();
    let config = AppConfig::load_layers(&path, None).unwrap();
    let (limits, warnings) = config.chat.reset_limits_setting();
    assert_eq!((limits.turns, limits.tokens), (2, 5000));
    assert!(warnings.is_empty());
    for value in ["0", "-5", "1.5", "\"x\""] {
        fs::write(
            &path,
            format!("[chat]\nreset_turns = {value}\nreset_tokens = {value}\n"),
        )
        .unwrap();
        let config = AppConfig::load_layers(&path, None).unwrap();
        let (limits, warnings) = config.chat.reset_limits_setting();
        assert_eq!((limits.turns, limits.tokens), (30, 140_000));
        assert_eq!(warnings.len(), 2);
        assert!(warnings
            .iter()
            .all(|text| text.contains("positive integer")));
    }
}
