//! Tests for LogMetadata type.

use super::super::*;
use ralph_core::stream::{IterationCosts, IterationMetadata, Usage};

#[test]
fn test_log_metadata_new() {
    let metadata = LogMetadata::new();
    assert!(metadata.is_empty());
    assert!(metadata.claude_session_id.is_none());
    assert!(metadata.model.is_none());
    assert!(metadata.cost_usd.is_none());
    assert!(metadata.duration_ms.is_none());
    assert!(metadata.usage.is_none());
}

#[test]
fn test_log_metadata_is_empty() {
    let empty = LogMetadata::default();
    assert!(empty.is_empty());

    let with_session_id = LogMetadata {
        claude_session_id: Some("abc".to_string()),
        ..Default::default()
    };
    assert!(!with_session_id.is_empty());

    let with_cost = LogMetadata {
        cost_usd: Some(0.05),
        ..Default::default()
    };
    assert!(!with_cost.is_empty());
}

#[test]
fn test_log_metadata_from_extracted() {
    let metadata = IterationMetadata {
        session_id: Some("abc-123".to_string()),
        model: Some("claude-opus-4-5".to_string()),
        tools: vec![],
    };

    let costs = IterationCosts {
        cost_usd: Some(0.05),
        duration_ms: Some(10000),
        usage: Some(Usage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_input_tokens: Some(0),
            cache_creation_input_tokens: Some(0),
        }),
    };

    let log_metadata = LogMetadata::from_extracted(metadata, costs);

    assert_eq!(log_metadata.claude_session_id, Some("abc-123".to_string()));
    assert_eq!(log_metadata.model, Some("claude-opus-4-5".to_string()));
    assert_eq!(log_metadata.cost_usd, Some(0.05));
    assert_eq!(log_metadata.duration_ms, Some(10000));
    assert!(log_metadata.usage.is_some());
    assert_eq!(log_metadata.usage.as_ref().unwrap().input_tokens, 100);
}

#[test]
fn test_log_metadata_serialization() {
    let metadata = LogMetadata {
        claude_session_id: Some("f5b6aaac-4316-454a-b086-a3f9e4351b1e".to_string()),
        model: Some("claude-opus-4-5-20251101".to_string()),
        cost_usd: Some(0.226354),
        duration_ms: Some(40966),
        usage: Some(Usage {
            input_tokens: 712,
            output_tokens: 2971,
            cache_read_input_tokens: Some(107476),
            cache_creation_input_tokens: Some(12504),
        }),
    };

    let toml_str = toml::to_string_pretty(&metadata).unwrap();

    assert!(toml_str.contains("claude_session_id = \"f5b6aaac-4316-454a-b086-a3f9e4351b1e\""));
    assert!(toml_str.contains("model = \"claude-opus-4-5-20251101\""));
    assert!(toml_str.contains("cost_usd = 0.226354"));
    assert!(toml_str.contains("duration_ms = 40966"));
    assert!(toml_str.contains("[usage]"));
    assert!(toml_str.contains("input_tokens = 712"));
    assert!(toml_str.contains("output_tokens = 2971"));
}

#[test]
fn test_log_metadata_deserialization() {
    let toml_str = r#"
        claude_session_id = "abc-123"
        model = "claude-opus-4-5"
        cost_usd = 0.05
        duration_ms = 10000

        [usage]
        input_tokens = 100
        output_tokens = 200
        cache_read_input_tokens = 0
        cache_creation_input_tokens = 0
    "#;

    let metadata: LogMetadata = toml::from_str(toml_str).unwrap();

    assert_eq!(metadata.claude_session_id, Some("abc-123".to_string()));
    assert_eq!(metadata.model, Some("claude-opus-4-5".to_string()));
    assert_eq!(metadata.cost_usd, Some(0.05));
    assert_eq!(metadata.duration_ms, Some(10000));
    assert!(metadata.usage.is_some());
}

#[test]
fn test_log_metadata_empty_fields_skipped() {
    let metadata = LogMetadata {
        claude_session_id: Some("abc".to_string()),
        model: None,
        cost_usd: None,
        duration_ms: None,
        usage: None,
    };

    let toml_str = toml::to_string(&metadata).unwrap();

    // Only claude_session_id should appear
    assert!(toml_str.contains("claude_session_id"));
    assert!(!toml_str.contains("model"));
    assert!(!toml_str.contains("cost_usd"));
    assert!(!toml_str.contains("duration_ms"));
    assert!(!toml_str.contains("usage"));
}
