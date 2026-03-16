//! Strategy configuration parsing and validation.
//!
//! Pure functions for parsing TOML strategy files and validating their
//! contents against known kinds and existing personas. Following the
//! Functional Core pattern, all functions operate on data provided as
//! arguments - no file I/O.

use std::collections::HashMap;

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
#[derive(Debug, Clone, Deserialize)]
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

/// Team strategy configuration from `.claude/strategy.toml`.
///
/// Defines the project's agent roster. Separate from per-strategy execution
/// configs in `.claude/strategies/*.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct TeamStrategy {
    /// Maps agent names to their markdown file paths (relative to project root).
    pub agents: HashMap<String, String>,
}

/// Parse `.claude/strategy.toml` content into a `TeamStrategy`.
///
/// Pure function — no I/O. Takes TOML content as a string.
pub fn parse_team_strategy(content: &str, path: &str) -> Result<TeamStrategy, StrategyError> {
    toml::from_str(content).map_err(|source| StrategyError::Parse {
        path: path.to_string(),
        source,
    })
}

/// Typed strategy kind, resolved from the `kind` string field.
///
/// Each variant maps 1:1 to a Rust implementation module in the shell crate.
/// New kinds require: (1) adding a variant here, (2) a case in `resolve_kind`,
/// (3) an implementation module in `crates/ralph/src/strategy/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyKind {
    /// PRD-driven iteration loop using a persona.
    PrdLoop,
    /// Human-driven conversation loop with agent collaboration.
    ConversationLoop,
}

/// Keyboard actions propagated from strategy execution.
///
/// Strategies report these to the caller so that keyboard controls
/// (soft stop, hard stop, pause) are preserved across the trait boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// Finish current iteration then exit.
    SoftStop,
    /// Immediately halt and save paused state.
    HardStop,
    /// Toggle pause/resume.
    Pause,
    /// Continue to next iteration.
    Continue,
}

/// Result of executing a strategy.
///
/// Returned by `Strategy::execute` to communicate the outcome of running
/// the strategy, including metrics, completion state, and any keyboard
/// action detected during execution.
#[derive(Debug)]
pub struct StrategyResult {
    /// Keyboard action detected during execution (if any).
    pub key_action: Option<KeyAction>,
    /// Session slug.
    pub slug: String,
    /// Number of iterations completed.
    pub iterations_completed: usize,
    /// Reason for completion (human-readable).
    pub completion_reason: Option<String>,
    /// Final count of pending stories.
    pub final_pending_stories: usize,
    /// Total cost in USD across all iterations.
    pub total_cost_usd: Option<f64>,
    /// Total duration in milliseconds across all iterations.
    pub total_duration_ms: Option<u64>,
    /// Total input tokens across all iterations.
    pub total_input_tokens: Option<u64>,
    /// Total output tokens across all iterations.
    pub total_output_tokens: Option<u64>,
}

/// Decision for what to do between strategy iterations.
///
/// Returned by `Strategy::between_iterations` to control the loop driver.
/// The `Orchestrate` variant carries directive strings that should be
/// resolved before continuing (e.g., invoking other personas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterationDecision {
    /// Continue to the next iteration.
    Continue,
    /// Resolve orchestration directives before continuing.
    Orchestrate(Vec<String>),
    /// Stop the strategy.
    Stop,
}

/// Resolve a `kind` string from TOML into a typed `StrategyKind`.
///
/// Pure function — no I/O. Returns `None` for unknown kinds.
///
/// # Examples
///
/// ```
/// use ralph_core::strategy::resolve_kind;
///
/// assert_eq!(resolve_kind("prd-loop"), Some(ralph_core::strategy::StrategyKind::PrdLoop));
/// assert_eq!(resolve_kind("unknown"), None);
/// ```
pub fn resolve_kind(kind: &str) -> Option<StrategyKind> {
    match kind {
        "prd-loop" => Some(StrategyKind::PrdLoop),
        "conversation-loop" => Some(StrategyKind::ConversationLoop),
        _ => None,
    }
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

    // =========================================================================
    // StrategyKind resolution tests
    // =========================================================================

    #[test]
    fn test_resolve_kind_prd_loop() {
        assert_eq!(resolve_kind("prd-loop"), Some(StrategyKind::PrdLoop));
    }

    #[test]
    fn test_resolve_kind_conversation_loop() {
        assert_eq!(
            resolve_kind("conversation-loop"),
            Some(StrategyKind::ConversationLoop)
        );
    }

    #[test]
    fn test_resolve_kind_unknown() {
        assert_eq!(resolve_kind("unknown"), None);
    }

    #[test]
    fn test_resolve_kind_empty() {
        assert_eq!(resolve_kind(""), None);
    }

    #[test]
    fn test_resolve_kind_case_sensitive() {
        assert_eq!(resolve_kind("PRD-LOOP"), None);
        assert_eq!(resolve_kind("Prd-Loop"), None);
    }

    // =========================================================================
    // KeyAction tests
    // =========================================================================

    #[test]
    fn test_key_action_variants_distinct() {
        assert_ne!(KeyAction::SoftStop, KeyAction::HardStop);
        assert_ne!(KeyAction::HardStop, KeyAction::Pause);
        assert_ne!(KeyAction::Pause, KeyAction::Continue);
        assert_ne!(KeyAction::Continue, KeyAction::SoftStop);
    }

    #[test]
    fn test_key_action_clone() {
        let action = KeyAction::SoftStop;
        let cloned = action;
        assert_eq!(action, cloned);
    }

    // =========================================================================
    // IterationDecision tests
    // =========================================================================

    #[test]
    fn test_iteration_decision_continue() {
        let decision = IterationDecision::Continue;
        assert_eq!(decision, IterationDecision::Continue);
    }

    #[test]
    fn test_iteration_decision_orchestrate() {
        let directives = vec!["ask architect".to_string()];
        let decision = IterationDecision::Orchestrate(directives.clone());
        assert_eq!(decision, IterationDecision::Orchestrate(directives));
    }

    #[test]
    fn test_iteration_decision_stop() {
        let decision = IterationDecision::Stop;
        assert_eq!(decision, IterationDecision::Stop);
    }

    #[test]
    fn test_iteration_decision_variants_distinct() {
        assert_ne!(IterationDecision::Continue, IterationDecision::Stop);
        assert_ne!(
            IterationDecision::Continue,
            IterationDecision::Orchestrate(vec![])
        );
    }

    // =========================================================================
    // TeamStrategy parsing tests
    // =========================================================================

    #[test]
    fn test_parse_team_strategy_valid() {
        let content = r#"
[agents]
architect = ".claude/agents/architect.md"
developer = ".claude/agents/developer.md"
"#;
        let strategy = parse_team_strategy(content, "test.toml").unwrap();
        assert_eq!(strategy.agents.len(), 2);
        assert_eq!(
            strategy.agents.get("architect"),
            Some(&".claude/agents/architect.md".to_string())
        );
        assert_eq!(
            strategy.agents.get("developer"),
            Some(&".claude/agents/developer.md".to_string())
        );
    }

    #[test]
    fn test_parse_team_strategy_empty_agents() {
        let content = r#"
[agents]
"#;
        let strategy = parse_team_strategy(content, "test.toml").unwrap();
        assert!(strategy.agents.is_empty());
    }

    #[test]
    fn test_parse_team_strategy_missing_agents_table() {
        let content = r#"
something_else = "value"
"#;
        let result = parse_team_strategy(content, "test.toml");
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_team_strategy_malformed_toml() {
        let content = "not valid [[[toml";
        let result = parse_team_strategy(content, "test.toml");
        assert!(matches!(result, Err(StrategyError::Parse { .. })));
    }

    #[test]
    fn test_parse_team_strategy_error_includes_path() {
        let content = "invalid [[[";
        let err = parse_team_strategy(content, "my/path.toml").unwrap_err();
        match err {
            StrategyError::Parse { path, .. } => assert_eq!(path, "my/path.toml"),
            other => panic!("Expected Parse error, got: {other:?}"),
        }
    }
}
