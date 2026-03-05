//! Strategy discovery, loading, and display.
//!
//! Imperative shell for strategy files: discovers `.toml` files from the
//! project's `.claude/strategies/` directory, reads and validates them
//! using the pure core functions, and formats results for display.

pub mod execute;
mod prd_loop;

use ralph_core::strategy::{
    parse_strategy, validate_personas, validate_prompt_aggregates, StrategyConfig, StrategyError,
    StrategyKind,
};
use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::persona;

/// Known strategy kinds supported by the runtime.
const KNOWN_KINDS: &[&str] = &["prd-loop"];

/// Errors from strategy I/O and validation in the shell layer.
#[derive(Debug, thiserror::Error)]
pub enum StrategyLoadError {
    /// Failed to read a strategy file from disk.
    #[error("Failed to read strategy file {path}: {source}")]
    ReadFile {
        path: String,
        source: std::io::Error,
    },

    /// A core validation error (parse, kind, persona, or prompt aggregate).
    #[error(transparent)]
    Validation(#[from] StrategyError),
}

/// A discovered strategy file (path only, not yet parsed).
pub struct StrategyDiscovery {
    pub path: PathBuf,
}

/// A fully loaded and validated strategy.
#[derive(Debug)]
pub struct LoadedStrategy {
    pub config: StrategyConfig,
    /// Resolved strategy kind (typed from config.kind string).
    pub kind: StrategyKind,
}

/// Discover strategy TOML files in `.claude/strategies/` under the project directory.
///
/// Returns one `StrategyDiscovery` per `.toml` file found.
/// Gracefully returns an empty vec if the directory does not exist.
pub fn discover_strategies(project_path: &Path) -> Vec<StrategyDiscovery> {
    let dir = project_path.join(".claude").join("strategies");

    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return vec![],
        Err(e) => {
            crate::warn::warn(format!(
                "Failed to read strategies directory {}: {e}",
                dir.display()
            ));
            return vec![];
        }
    };

    let mut discoveries = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            discoveries.push(StrategyDiscovery { path });
        }
    }

    discoveries
}

/// Read, parse, and fully validate a single strategy file.
///
/// Validates prompt aggregates and persona references against the provided
/// list of available persona names, and resolves the strategy kind.
pub fn load_and_validate_strategy(
    path: &Path,
    available_persona_names: &[&str],
) -> Result<(StrategyConfig, StrategyKind), StrategyLoadError> {
    let content = std::fs::read_to_string(path).map_err(|source| StrategyLoadError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;

    let path_str = path.display().to_string();
    let config = parse_strategy(&content, &path_str)?;
    validate_prompt_aggregates(&config, &path_str)?;
    validate_personas(&config, available_persona_names, &path_str)?;

    let kind = ralph_core::strategy::resolve_kind(&config.kind).ok_or_else(|| {
        StrategyError::UnknownKind {
            path: path_str.clone(),
            kind: config.kind.clone(),
            known: KNOWN_KINDS.iter().map(|s| s.to_string()).collect(),
        }
    })?;

    Ok((config, kind))
}

/// Discover, load, and validate all strategies in the project.
///
/// Discovers persona names via `persona::discover_personas` and then
/// loads each strategy file, validating against the known kinds and
/// available personas.
pub fn load_all_strategies(project_path: &Path) -> Result<Vec<LoadedStrategy>, StrategyLoadError> {
    let personas = persona::discover_personas(project_path);
    let persona_names: Vec<&str> = personas.iter().map(|p| p.name.as_str()).collect();

    let discoveries = discover_strategies(project_path);
    let mut loaded = Vec::new();

    for discovery in discoveries {
        let (config, kind) = load_and_validate_strategy(&discovery.path, &persona_names)?;
        loaded.push(LoadedStrategy { config, kind });
    }

    Ok(loaded)
}

/// Find a loaded strategy by name, returning an error listing available strategies if not found.
pub fn find_strategy_by_name<'a>(
    strategies: &'a [LoadedStrategy],
    name: &str,
) -> Result<&'a LoadedStrategy, String> {
    strategies
        .iter()
        .find(|s| s.config.name == name)
        .ok_or_else(|| {
            let available: Vec<&str> = strategies.iter().map(|s| s.config.name.as_str()).collect();
            if available.is_empty() {
                format!(
                    "Unknown strategy '{name}'. No strategies found.\n\
                     Create strategy files in .claude/strategies/<name>.toml."
                )
            } else {
                format!(
                    "Unknown strategy '{name}'. Available strategies: {}",
                    available.join(", ")
                )
            }
        })
}

/// Format a list of loaded strategies as a columnar table.
///
/// Pure function — no I/O. Returns a ready-to-print string.
pub fn format_strategy_list(strategies: &[LoadedStrategy]) -> String {
    if strategies.is_empty() {
        return "No strategies found.\n\n\
                Create strategy files in .claude/strategies/<name>.toml."
            .to_string();
    }

    let name_width = strategies
        .iter()
        .map(|s| s.config.name.len())
        .max()
        .unwrap_or(4)
        .max(4); // "NAME" header

    let kind_width = strategies
        .iter()
        .map(|s| s.config.kind.len())
        .max()
        .unwrap_or(4)
        .max(4); // "KIND" header

    let persona_width = strategies
        .iter()
        .map(|s| s.config.primary_persona.len())
        .max()
        .unwrap_or(15)
        .max(15); // "PRIMARY_PERSONA" header

    let mut out = String::new();

    // Header
    let _ = writeln!(
        out,
        "{:<name_width$}  {:<kind_width$}  {:<persona_width$}  DESCRIPTION",
        "NAME", "KIND", "PRIMARY_PERSONA",
    );
    let _ = writeln!(
        out,
        "{:<name_width$}  {:<kind_width$}  {:<persona_width$}  {}",
        "\u{2500}".repeat(name_width),
        "\u{2500}".repeat(kind_width),
        "\u{2500}".repeat(persona_width),
        "\u{2500}".repeat(11),
    );

    for s in strategies {
        let _ = writeln!(
            out,
            "{:<name_width$}  {:<kind_width$}  {:<persona_width$}  {}",
            s.config.name, s.config.kind, s.config.primary_persona, s.config.description,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(name: &str, kind: &str, persona: &str, desc: &str) -> StrategyConfig {
        StrategyConfig {
            name: name.to_string(),
            description: desc.to_string(),
            kind: kind.to_string(),
            primary_persona: persona.to_string(),
            available_personas: vec![],
            prompt_aggregates: vec![],
        }
    }

    fn make_loaded(name: &str, kind_str: &str, persona: &str, desc: &str) -> LoadedStrategy {
        LoadedStrategy {
            config: make_config(name, kind_str, persona, desc),
            kind: ralph_core::strategy::resolve_kind(kind_str)
                .expect("tests must use a known strategy kind"),
        }
    }

    #[test]
    fn test_format_empty() {
        let output = format_strategy_list(&[]);
        assert!(output.contains("No strategies found"));
        assert!(output.contains(".claude/strategies/"));
    }

    #[test]
    fn test_format_single() {
        let strategies = vec![make_loaded(
            "iterative-dev",
            "prd-loop",
            "dev",
            "Iterative development",
        )];
        let output = format_strategy_list(&strategies);
        assert!(output.contains("NAME"));
        assert!(output.contains("KIND"));
        assert!(output.contains("PRIMARY_PERSONA"));
        assert!(output.contains("DESCRIPTION"));
        assert!(output.contains("iterative-dev"));
        assert!(output.contains("prd-loop"));
        assert!(output.contains("dev"));
        assert!(output.contains("Iterative development"));
    }

    #[test]
    fn test_format_multiple() {
        let strategies = vec![
            make_loaded("alpha", "prd-loop", "dev", "First strategy"),
            make_loaded("beta", "prd-loop", "reviewer", "Second strategy"),
        ];
        let output = format_strategy_list(&strategies);
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
        assert!(output.contains("First strategy"));
        assert!(output.contains("Second strategy"));
    }

    #[test]
    fn test_format_column_alignment() {
        let strategies = vec![
            make_loaded("short", "prd-loop", "dev", "Short name"),
            make_loaded(
                "a-very-long-strategy-name",
                "prd-loop",
                "reviewer",
                "Long name",
            ),
        ];
        let output = format_strategy_list(&strategies);
        // Both rows should contain all data
        assert!(output.contains("short"));
        assert!(output.contains("a-very-long-strategy-name"));
        // Header separator should be present
        assert!(output.contains("\u{2500}"));
    }

    // =========================================================================
    // find_strategy_by_name tests
    // =========================================================================

    #[test]
    fn test_find_strategy_by_name_found() {
        let strategies = vec![
            make_loaded("alpha", "prd-loop", "dev", "First"),
            make_loaded("beta", "prd-loop", "reviewer", "Second"),
        ];
        let result = find_strategy_by_name(&strategies, "beta");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().config.name, "beta");
    }

    #[test]
    fn test_find_strategy_by_name_not_found() {
        let strategies = vec![
            make_loaded("alpha", "prd-loop", "dev", "First"),
            make_loaded("beta", "prd-loop", "reviewer", "Second"),
        ];
        let err = find_strategy_by_name(&strategies, "gamma").unwrap_err();
        assert!(err.contains("Unknown strategy 'gamma'"));
        assert!(err.contains("alpha"));
        assert!(err.contains("beta"));
    }

    #[test]
    fn test_find_strategy_by_name_empty_list() {
        let strategies: Vec<LoadedStrategy> = vec![];
        let err = find_strategy_by_name(&strategies, "anything").unwrap_err();
        assert!(err.contains("No strategies found"));
        assert!(err.contains(".claude/strategies/"));
    }
}
