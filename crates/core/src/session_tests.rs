//! Extended session tests (persona, metadata, index mutations).
//!
//! Split from session.rs to keep the main file under 1000 lines.

use super::*;
use std::path::PathBuf;

#[test]
fn session_entry_deserializes_without_persona_field() {
    let toml = r#"
slug = "quiet-mountain"
project = "/tmp/test"
started_at = "2024-01-01T00:00:00Z"
iterations = 3
outcome = "completed"
"#;
    let entry: SessionEntry = toml::from_str(toml).unwrap();
    assert!(entry.persona.is_none());
}

#[test]
fn session_entry_deserializes_with_persona_field() {
    let toml = r#"
slug = "quiet-mountain"
project = "/tmp/test"
started_at = "2024-01-01T00:00:00Z"
iterations = 1
outcome = "in_progress"
persona = "coach"
"#;
    let entry: SessionEntry = toml::from_str(toml).unwrap();
    assert_eq!(entry.persona.as_deref(), Some("coach"));
}

#[test]
fn find_most_recent_for_persona_filters_by_persona() {
    let project = PathBuf::from("/tmp/project");
    let mut index = SessionsIndex::new();

    let mut ask_entry = SessionEntry::new("ask-session".to_string(), project.clone());
    ask_entry.started_at = Utc::now() - chrono::Duration::hours(2);
    index.add_session(ask_entry);

    let mut coach_entry = SessionEntry::new_with_persona(
        "coach-session".to_string(),
        project.clone(),
        "coach".to_string(),
    );
    coach_entry.started_at = Utc::now() - chrono::Duration::hours(1);
    index.add_session(coach_entry);

    // Searching for persona=None finds only the ask session
    let result = index.find_most_recent_for_persona(&project, None);
    assert_eq!(result.map(|s| s.slug.as_str()), Some("ask-session"));

    // Searching for persona=Some("coach") finds only the coach session
    let result = index.find_most_recent_for_persona(&project, Some("coach"));
    assert_eq!(result.map(|s| s.slug.as_str()), Some("coach-session"));

    // Searching for a non-existent persona returns None
    let result = index.find_most_recent_for_persona(&project, Some("nonexistent"));
    assert!(result.is_none());
}

#[test]
fn find_most_recent_for_persona_returns_most_recent() {
    let project = PathBuf::from("/tmp/project");
    let mut index = SessionsIndex::new();

    let mut older = SessionEntry::new_with_persona(
        "older-session".to_string(),
        project.clone(),
        "coach".to_string(),
    );
    older.started_at = Utc::now() - chrono::Duration::hours(2);
    index.add_session(older);

    let newer = SessionEntry::new_with_persona(
        "newer-session".to_string(),
        project.clone(),
        "coach".to_string(),
    );
    index.add_session(newer);

    let result = index.find_most_recent_for_persona(&project, Some("coach"));
    assert_eq!(result.map(|s| s.slug.as_str()), Some("newer-session"));
}

#[test]
fn find_most_recent_for_persona_ignores_other_projects() {
    let project_a = PathBuf::from("/tmp/project-a");
    let project_b = PathBuf::from("/tmp/project-b");
    let mut index = SessionsIndex::new();

    index.add_session(SessionEntry::new_with_persona(
        "a-session".to_string(),
        project_a.clone(),
        "coach".to_string(),
    ));
    index.add_session(SessionEntry::new_with_persona(
        "b-session".to_string(),
        project_b.clone(),
        "coach".to_string(),
    ));

    let result = index.find_most_recent_for_persona(&project_a, Some("coach"));
    assert_eq!(result.map(|s| s.slug.as_str()), Some("a-session"));
}

#[test]
fn session_metadata_round_trip_with_persona() {
    let meta = SessionMetadata::new_with_persona(
        "test-slug".to_string(),
        PathBuf::from("/tmp/project"),
        Some("hello".to_string()),
        "coach".to_string(),
    );
    let toml_str = meta.to_toml().unwrap();
    let parsed: SessionMetadata = toml::from_str(&toml_str).unwrap();
    assert_eq!(parsed.persona.as_deref(), Some("coach"));
    assert_eq!(parsed.slug, "test-slug");
}

#[test]
fn session_metadata_from_entry_preserves_persona() {
    let entry = SessionEntry::new_with_persona(
        "test-slug".to_string(),
        PathBuf::from("/tmp"),
        "reviewer".to_string(),
    );
    let meta = SessionMetadata::from(&entry);
    assert_eq!(meta.persona.as_deref(), Some("reviewer"));
}

#[test]
fn session_entry_new_with_persona() {
    let entry = SessionEntry::new_with_persona(
        "test-slug".to_string(),
        PathBuf::from("/tmp"),
        "reviewer".to_string(),
    );
    assert_eq!(entry.persona.as_deref(), Some("reviewer"));
    assert_eq!(entry.outcome, SessionOutcome::InProgress);
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
