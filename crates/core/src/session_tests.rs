//! Extended session tests (metadata, index mutations).
//!
//! Split from session.rs to keep the main file under 1000 lines.

use super::*;
use std::path::PathBuf;

#[test]
fn session_entry_ignores_legacy_persona_field() {
    let toml = r#"
slug = "quiet-mountain"
project = "/tmp/test"
started_at = "2024-01-01T00:00:00Z"
iterations = 1
outcome = "in_progress"
persona = "coach"
"#;
    let entry: SessionEntry = toml::from_str(toml).unwrap();
    assert_eq!(entry.slug, "quiet-mountain");
}

#[test]
fn find_most_recent_for_project_picks_latest_in_project() {
    let project = PathBuf::from("/tmp/project");
    let mut index = SessionsIndex::new();
    let mut older = SessionEntry::new("older".to_string(), project.clone());
    older.started_at = Utc::now() - chrono::Duration::hours(2);
    index.add_session(older);
    index.add_session(SessionEntry::new("newer".to_string(), project.clone()));
    index.add_session(SessionEntry::new(
        "other".to_string(),
        PathBuf::from("/tmp/x"),
    ));
    let result = index.find_most_recent_for_project(&project);
    assert_eq!(result.map(|s| s.slug.as_str()), Some("newer"));
}

#[test]
fn session_metadata_toml_roundtrip() {
    // Without prompt - verify it's not serialized
    let meta = SessionMetadata::new("calm-ocean".to_string(), PathBuf::from("/test"), None);
    let toml_str = meta.to_toml().unwrap();
    assert!(
        !toml_str.contains("prompt"),
        "prompt should not appear when None"
    );
    let parsed = SessionMetadata::from_toml(&toml_str).unwrap();
    assert_eq!(parsed.slug, "calm-ocean");
    assert_eq!(parsed.prompt, None);

    // With prompt - verify roundtrip preserves it
    let prompt = "Work on feature".to_string();
    let meta = SessionMetadata::new(
        "swift-wind".to_string(),
        PathBuf::from("/test"),
        Some(prompt.clone()),
    );
    let toml_str = meta.to_toml().unwrap();
    assert!(toml_str.contains("prompt = "));
    let parsed = SessionMetadata::from_toml(&toml_str).unwrap();
    assert_eq!(parsed.prompt, Some(prompt));
}

#[test]
fn session_metadata_from_entry() {
    let entry = SessionEntry::new("gentle-breeze".to_string(), PathBuf::from("/project"));
    let meta = SessionMetadata::from(&entry);

    assert_eq!(meta.slug, entry.slug);
    assert_eq!(meta.project, entry.project);
    assert_eq!(meta.iterations, entry.iterations);
    assert_eq!(meta.outcome, entry.outcome);
}

#[test]
fn sessions_index_find_by_slug_mut() {
    let mut index = SessionsIndex::new();
    index.add_session(SessionEntry::new(
        "test-slug".to_string(),
        PathBuf::from("/test"),
    ));

    // Modify through mutable reference
    if let Some(entry) = index.find_by_slug_mut("test-slug") {
        entry.iterations = 5;
        entry.outcome = SessionOutcome::Completed;
    }

    // Verify modification
    let entry = index.find_by_slug("test-slug").unwrap();
    assert_eq!(entry.iterations, 5);
    assert_eq!(entry.outcome, SessionOutcome::Completed);
}
