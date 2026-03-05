//! Strategy configuration parsing and validation.
//!
//! Pure functions for parsing TOML strategy files and validating their
//! contents against known kinds and existing personas. Following the
//! Functional Core pattern, all functions operate on data provided as
//! arguments - no file I/O.

use serde::Deserialize;

/// Error type for strategy parsing and validation operations.
#[derive(thiserror::Error, Debug)]
pub enum StrategyError {
    /// TOML parsing failed.
    #[error("Failed to parse strategy at {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },

    /// The `kind` field does not map to a known implementation.
    #[error("Unknown strategy kind `{kind}` in {path} (known: {known:?})")]
    UnknownKind {
        path: String,
        kind: String,
        known: Vec<String>,
    },

    /// A referenced persona does not have an agent file.
    #[error("Persona `{persona}` referenced in {path} not found")]
    PersonaNotFound { path: String, persona: String },

    /// An entry in `prompt_aggregates` is empty.
    #[error("Empty prompt_aggregates entry at index {index} in {path}")]
    EmptyPromptAggregate { path: String, index: usize },
}

/// Parsed strategy configuration from a TOML file.
#[derive(Debug, Deserialize)]
pub struct StrategyConfig {
    /// Human-readable name for this strategy.
    pub name: String,
    /// Description of what the strategy does.
    pub description: String,
    /// Maps to a Rust implementation.
    pub kind: String,
    /// The persona that executes iterations.
    pub primary_persona: String,
    /// Additional personas available for orchestration.
    #[serde(default)]
    pub available_personas: Vec<String>,
    /// Additional instruction aggregates.
    #[serde(default)]
    pub prompt_aggregates: Vec<String>,
}

/// Parse TOML content into a `StrategyConfig`.
///
/// This is a pure function - it takes TOML content as a string and returns
/// the parsed config. File I/O is handled at the shell layer.
///
/// # Arguments
///
/// * `content` - The raw TOML content of the strategy file
/// * `path` - File path used in error messages
pub fn parse_strategy(content: &str, path: &str) -> Result<StrategyConfig, StrategyError> {
    toml::from_str(content).map_err(|source| StrategyError::Parse {
        path: path.to_string(),
        source,
    })
}

/// Validate that the strategy's `kind` is in the list of known kinds.
pub fn validate_kind(
    config: &StrategyConfig,
    known_kinds: &[&str],
    path: &str,
) -> Result<(), StrategyError> {
    if known_kinds.contains(&config.kind.as_str()) {
        Ok(())
    } else {
        Err(StrategyError::UnknownKind {
            path: path.to_string(),
            kind: config.kind.clone(),
            known: known_kinds.iter().map(|s| s.to_string()).collect(),
        })
    }
}

/// Validate that `primary_persona` and all `available_personas` exist.
pub fn validate_personas(
    config: &StrategyConfig,
    existing_personas: &[&str],
    path: &str,
) -> Result<(), StrategyError> {
    if !existing_personas.contains(&config.primary_persona.as_str()) {
        return Err(StrategyError::PersonaNotFound {
            path: path.to_string(),
            persona: config.primary_persona.clone(),
        });
    }

    for persona in &config.available_personas {
        if !existing_personas.contains(&persona.as_str()) {
            return Err(StrategyError::PersonaNotFound {
                path: path.to_string(),
                persona: persona.clone(),
            });
        }
    }

    Ok(())
}

/// Validate that no entries in `prompt_aggregates` are empty strings.
pub fn validate_prompt_aggregates(
    config: &StrategyConfig,
    path: &str,
) -> Result<(), StrategyError> {
    for (index, entry) in config.prompt_aggregates.iter().enumerate() {
        if entry.is_empty() {
            return Err(StrategyError::EmptyPromptAggregate {
                path: path.to_string(),
                index,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = ".claude/strategies/test.toml";

    fn full_toml() -> &'static str {
        r#"
name = "iterative-dev"
description = "Iterative development with PRD"
kind = "prd-loop"
primary_persona = "dev"
available_personas = ["reviewer", "tester"]
prompt_aggregates = ["Always run tests", "Use conventional commits"]
"#
    }

    // =========================================================================
    // Parsing tests
    // =========================================================================

    #[test]
    fn test_parse_all_fields() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        assert_eq!(config.name, "iterative-dev");
        assert_eq!(config.description, "Iterative development with PRD");
        assert_eq!(config.kind, "prd-loop");
        assert_eq!(config.primary_persona, "dev");
        assert_eq!(config.available_personas, vec!["reviewer", "tester"]);
        assert_eq!(
            config.prompt_aggregates,
            vec!["Always run tests", "Use conventional commits"]
        );
    }

    #[test]
    fn test_parse_optional_fields_omitted() {
        let content = r#"
name = "minimal"
description = "A minimal strategy"
kind = "simple"
primary_persona = "dev"
"#;
        let config = parse_strategy(content, PATH).unwrap();
        assert_eq!(config.name, "minimal");
        assert!(config.available_personas.is_empty());
        assert!(config.prompt_aggregates.is_empty());
    }

    #[test]
    fn test_parse_missing_name() {
        let content = r#"
description = "No name"
kind = "simple"
primary_persona = "dev"
"#;
        let result = parse_strategy(content, PATH);
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_missing_description() {
        let content = r#"
name = "no-desc"
kind = "simple"
primary_persona = "dev"
"#;
        let result = parse_strategy(content, PATH);
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_missing_kind() {
        let content = r#"
name = "no-kind"
description = "Missing kind"
primary_persona = "dev"
"#;
        let result = parse_strategy(content, PATH);
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_missing_primary_persona() {
        let content = r#"
name = "no-persona"
description = "Missing primary persona"
kind = "simple"
"#;
        let result = parse_strategy(content, PATH);
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_malformed_toml() {
        let content = "this is not valid [[[toml";
        let result = parse_strategy(content, PATH);
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_error_includes_path() {
        let content = "invalid toml [[[";
        let err = parse_strategy(content, PATH).unwrap_err();
        match err {
            StrategyError::Parse { path, .. } => assert_eq!(path, PATH),
            other => panic!("Expected Parse error, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_error_message_includes_field_name() {
        let content = r#"
description = "No name"
kind = "simple"
primary_persona = "dev"
"#;
        let err = parse_strategy(content, PATH).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains(PATH), "Error should contain file path");
        assert!(msg.contains("name"), "Error should mention missing field");
    }

    // =========================================================================
    // Kind validation tests
    // =========================================================================

    #[test]
    fn test_validate_known_kind() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        let known = &["prd-loop", "simple"];
        assert!(validate_kind(&config, known, PATH).is_ok());
    }

    #[test]
    fn test_validate_unknown_kind() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        let known = &["simple", "one-shot"];
        let err = validate_kind(&config, known, PATH).unwrap_err();
        match err {
            StrategyError::UnknownKind { kind, known: k, .. } => {
                assert_eq!(kind, "prd-loop");
                assert_eq!(k, vec!["simple", "one-shot"]);
            }
            other => panic!("Expected UnknownKind, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_kind_empty_known_list() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        let known: &[&str] = &[];
        assert!(matches!(
            validate_kind(&config, known, PATH),
            Err(StrategyError::UnknownKind { .. })
        ));
    }

    // =========================================================================
    // Persona validation tests
    // =========================================================================

    #[test]
    fn test_validate_personas_all_exist() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        let existing = &["dev", "reviewer", "tester", "pm"];
        assert!(validate_personas(&config, existing, PATH).is_ok());
    }

    #[test]
    fn test_validate_primary_persona_not_found() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        let existing = &["reviewer", "tester"]; // missing "dev"
        let err = validate_personas(&config, existing, PATH).unwrap_err();
        match err {
            StrategyError::PersonaNotFound { persona, .. } => {
                assert_eq!(persona, "dev");
            }
            other => panic!("Expected PersonaNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_available_persona_not_found() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        let existing = &["dev", "reviewer"]; // missing "tester"
        let err = validate_personas(&config, existing, PATH).unwrap_err();
        match err {
            StrategyError::PersonaNotFound { persona, .. } => {
                assert_eq!(persona, "tester");
            }
            other => panic!("Expected PersonaNotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_personas_no_available() {
        let content = r#"
name = "solo"
description = "No available personas"
kind = "simple"
primary_persona = "dev"
"#;
        let config = parse_strategy(content, PATH).unwrap();
        let existing = &["dev"];
        assert!(validate_personas(&config, existing, PATH).is_ok());
    }

    // =========================================================================
    // Prompt aggregates validation tests
    // =========================================================================

    #[test]
    fn test_validate_prompt_aggregates_valid() {
        let config = parse_strategy(full_toml(), PATH).unwrap();
        assert!(validate_prompt_aggregates(&config, PATH).is_ok());
    }

    #[test]
    fn test_validate_prompt_aggregates_empty_entry() {
        let content = r#"
name = "bad-agg"
description = "Has empty aggregate"
kind = "simple"
primary_persona = "dev"
prompt_aggregates = ["valid", "", "also valid"]
"#;
        let config = parse_strategy(content, PATH).unwrap();
        let err = validate_prompt_aggregates(&config, PATH).unwrap_err();
        match err {
            StrategyError::EmptyPromptAggregate { index, .. } => {
                assert_eq!(index, 1);
            }
            other => panic!("Expected EmptyPromptAggregate, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_prompt_aggregates_empty_list_is_valid() {
        let content = r#"
name = "no-agg"
description = "No aggregates"
kind = "simple"
primary_persona = "dev"
prompt_aggregates = []
"#;
        let config = parse_strategy(content, PATH).unwrap();
        assert!(validate_prompt_aggregates(&config, PATH).is_ok());
    }

    #[test]
    fn test_validate_prompt_aggregates_omitted_is_valid() {
        let content = r#"
name = "no-agg"
description = "Omitted aggregates"
kind = "simple"
primary_persona = "dev"
"#;
        let config = parse_strategy(content, PATH).unwrap();
        assert!(validate_prompt_aggregates(&config, PATH).is_ok());
    }

    #[test]
    fn test_validate_prompt_aggregates_first_entry_empty() {
        let content = r#"
name = "bad"
description = "First is empty"
kind = "simple"
primary_persona = "dev"
prompt_aggregates = [""]
"#;
        let config = parse_strategy(content, PATH).unwrap();
        let err = validate_prompt_aggregates(&config, PATH).unwrap_err();
        match err {
            StrategyError::EmptyPromptAggregate { index, .. } => {
                assert_eq!(index, 0);
            }
            other => panic!("Expected EmptyPromptAggregate, got: {other:?}"),
        }
    }
}
