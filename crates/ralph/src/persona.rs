//! Persona command — agent file resolution, discovery, and display.
//!
//! Verifies agent files exist, discovers available personas from both
//! project-local and user-level directories, and formats them for display.

use ralph_core::persona::{
    deduplicate_personas, parse_persona_frontmatter, PersonaInfo, PersonaSource,
};
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Errors specific to persona resolution.
#[derive(Debug, thiserror::Error)]
pub enum PersonaError {
    #[error("Agent file not found for persona '{name}'. Expected at: {path}\n\nCreate the agent file or check the persona name.")]
    AgentFileNotFound { name: String, path: String },

    #[error("Persona '{name}' is not defined in the project strategy.\n\nAvailable agents: {available}\n\nAdd it to .claude/strategy.toml [agents] table.")]
    NotInStrategy { name: String, available: String },
}

/// Strategy-aware agent file resolution.
///
/// When a `TeamStrategy` is provided, resolves from its `[agents]` table
/// exclusively. When `None`, falls back to project/user directory scan.
pub fn verify_agent_file_with_strategy(
    persona_name: &str,
    project_path: &Path,
    strategy: Option<&ralph_core::strategy::TeamStrategy>,
) -> Result<PathBuf, PersonaError> {
    if let Some(team) = strategy {
        return resolve_from_strategy(persona_name, project_path, team);
    }

    resolve_from_directories(persona_name, project_path)
}

/// Resolve an agent from the strategy's [agents] table.
fn resolve_from_strategy(
    persona_name: &str,
    project_path: &Path,
    strategy: &ralph_core::strategy::TeamStrategy,
) -> Result<PathBuf, PersonaError> {
    match strategy.agents.get(persona_name) {
        Some(relative_path) => {
            let agent_path = project_path.join(relative_path);
            if agent_path.exists() {
                Ok(agent_path)
            } else {
                Err(PersonaError::AgentFileNotFound {
                    name: persona_name.to_string(),
                    path: agent_path.display().to_string(),
                })
            }
        }
        None => {
            let mut available: Vec<&str> = strategy.agents.keys().map(|s| s.as_str()).collect();
            available.sort();
            Err(PersonaError::NotInStrategy {
                name: persona_name.to_string(),
                available: available.join(", "),
            })
        }
    }
}

/// Resolve an agent from project-local and user-level directories (legacy path).
fn resolve_from_directories(
    persona_name: &str,
    project_path: &Path,
) -> Result<PathBuf, PersonaError> {
    let project_agent = project_path
        .join(".claude")
        .join("agents")
        .join(format!("{}.md", persona_name));

    if project_agent.exists() {
        return Ok(project_agent);
    }

    if let Some(home) = dirs::home_dir() {
        let user_agent = home
            .join(".claude")
            .join("agents")
            .join(format!("{}.md", persona_name));

        if user_agent.exists() {
            return Ok(user_agent);
        }
    }

    Err(PersonaError::AgentFileNotFound {
        name: persona_name.to_string(),
        path: project_agent.display().to_string(),
    })
}

/// Scan a directory for agent `.md` files and parse their frontmatter.
///
/// Gracefully handles missing directories and unreadable files.
fn scan_agents_dir(dir: &Path, source: PersonaSource) -> Vec<PersonaInfo> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return vec![],
    };

    let mut personas = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(info) = parse_persona_frontmatter(&content, source) {
                personas.push(info);
            }
        }
    }

    personas
}

/// Discover all available personas from project-local and user-level directories.
///
/// Scans both locations and deduplicates (project takes precedence).
pub fn discover_personas(project_path: &Path) -> Vec<PersonaInfo> {
    let project_dir = project_path.join(".claude").join("agents");
    let project_personas = scan_agents_dir(&project_dir, PersonaSource::Project);

    let user_personas = match dirs::home_dir() {
        Some(home) => {
            let user_dir = home.join(".claude").join("agents");
            scan_agents_dir(&user_dir, PersonaSource::User)
        }
        None => vec![],
    };

    deduplicate_personas(project_personas, user_personas)
}

/// Discover personas with strategy awareness.
///
/// When a `TeamStrategy` is provided, reads agent files from the strategy's
/// paths and tags them as `PersonaSource::Strategy`. Does NOT fall back to
/// directory scanning.
///
/// When no strategy is provided, delegates to `discover_personas` for
/// backward-compatible behavior.
pub fn discover_personas_with_strategy(
    project_path: &Path,
    strategy: Option<&ralph_core::strategy::TeamStrategy>,
) -> Vec<PersonaInfo> {
    match strategy {
        Some(team) => discover_strategy_personas(project_path, team),
        None => discover_personas(project_path),
    }
}

/// Discover personas from a team strategy's [agents] table.
///
/// Reads each referenced agent file, parses frontmatter, and tags
/// with `PersonaSource::Strategy`. Files that don't exist or lack
/// valid frontmatter are silently skipped.
fn discover_strategy_personas(
    project_path: &Path,
    strategy: &ralph_core::strategy::TeamStrategy,
) -> Vec<PersonaInfo> {
    let mut personas = Vec::new();

    let mut entries: Vec<_> = strategy.agents.iter().collect();
    entries.sort_by_key(|(name, _)| name.as_str());

    for (name, relative_path) in entries {
        let agent_path = project_path.join(relative_path);
        if let Ok(content) = std::fs::read_to_string(&agent_path) {
            if let Some(mut info) = parse_persona_frontmatter(&content, PersonaSource::Strategy) {
                // Use the strategy key as the name, not the frontmatter name,
                // in case they differ.
                info.name = name.clone();
                personas.push(info);
            }
        }
    }

    personas
}

/// Format a list of personas as a columnar table.
///
/// Pure function — no I/O. Returns a ready-to-print string.
pub fn format_persona_list(personas: &[PersonaInfo]) -> String {
    if personas.is_empty() {
        return "No personas found.\n\n\
                Create agent files in .claude/agents/<name>.md (project) \
                or ~/.claude/agents/<name>.md (user)."
            .to_string();
    }

    let name_width = personas
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(4)
        .max(4); // "NAME" header

    let source_width = PersonaSource::Strategy.as_str().len(); // longest source label

    let mut out = String::new();

    // Header
    let _ = writeln!(
        out,
        "{:<name_width$}  {:<source_width$}  DESCRIPTION",
        "NAME", "SOURCE",
    );
    let _ = writeln!(
        out,
        "{:<name_width$}  {:<source_width$}  {}",
        "─".repeat(name_width),
        "─".repeat(source_width),
        "─".repeat(11),
    );

    for p in personas {
        let _ = writeln!(
            out,
            "{:<name_width$}  {:<source_width$}  {}",
            p.name,
            p.source.as_str(),
            p.description,
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::strategy::TeamStrategy;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn make_strategy(entries: &[(&str, &str)]) -> TeamStrategy {
        let agents = entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>();
        TeamStrategy { agents }
    }

    fn make_temp_agent(dir: &TempDir, name: &str, description: &str) {
        let agents_dir = dir.path().join(".claude").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(
            agents_dir.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: {description}\n---\n"),
        )
        .unwrap();
    }

    #[test]
    fn test_format_empty() {
        let output = format_persona_list(&[]);
        assert!(output.contains("No personas found"));
        assert!(output.contains(".claude/agents/"));
    }

    #[test]
    fn test_format_single() {
        let personas = vec![PersonaInfo {
            name: "dev".to_string(),
            description: "Development assistant".to_string(),
            source: PersonaSource::Project,
        }];
        let output = format_persona_list(&personas);
        assert!(output.contains("NAME"));
        assert!(output.contains("SOURCE"));
        assert!(output.contains("DESCRIPTION"));
        assert!(output.contains("dev"));
        assert!(output.contains("project"));
        assert!(output.contains("Development assistant"));
    }

    #[test]
    fn test_format_multiple() {
        let personas = vec![
            PersonaInfo {
                name: "alpha".to_string(),
                description: "First".to_string(),
                source: PersonaSource::Project,
            },
            PersonaInfo {
                name: "beta".to_string(),
                description: "Second".to_string(),
                source: PersonaSource::User,
            },
        ];
        let output = format_persona_list(&personas);
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
        assert!(output.contains("First"));
        assert!(output.contains("Second"));
    }

    #[test]
    fn test_format_source_indicator() {
        let personas = vec![
            PersonaInfo {
                name: "proj".to_string(),
                description: "Project one".to_string(),
                source: PersonaSource::Project,
            },
            PersonaInfo {
                name: "usr".to_string(),
                description: "User one".to_string(),
                source: PersonaSource::User,
            },
        ];
        let output = format_persona_list(&personas);
        assert!(output.contains("project"));
        assert!(output.contains("user"));
    }

    // =========================================================================
    // Strategy-aware resolution tests (Task 5.5)
    // =========================================================================

    #[test]
    fn test_resolve_from_strategy_found() {
        let dir = TempDir::new().unwrap();
        make_temp_agent(&dir, "architect", "test");

        let strategy = make_strategy(&[("architect", ".claude/agents/architect.md")]);

        let result = resolve_from_strategy("architect", dir.path(), &strategy);
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with("architect.md"));
    }

    #[test]
    fn test_resolve_from_strategy_not_in_table() {
        let strategy = make_strategy(&[("architect", ".claude/agents/architect.md")]);

        let result = resolve_from_strategy("developer", Path::new("/tmp"), &strategy);
        assert!(result.is_err());
        match result.unwrap_err() {
            PersonaError::NotInStrategy { name, available } => {
                assert_eq!(name, "developer");
                assert!(available.contains("architect"));
            }
            other => panic!("Expected NotInStrategy, got: {other:?}"),
        }
    }

    #[test]
    fn test_resolve_from_strategy_file_missing() {
        let dir = TempDir::new().unwrap();
        // Don't create the file -- it's in the table but missing on disk

        let strategy = make_strategy(&[("architect", ".claude/agents/architect.md")]);

        let result = resolve_from_strategy("architect", dir.path(), &strategy);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PersonaError::AgentFileNotFound { .. }
        ));
    }

    #[test]
    fn test_verify_with_strategy_none_falls_through() {
        let dir = TempDir::new().unwrap();
        make_temp_agent(&dir, "dev", "test");

        // No strategy passed -- should fall back to directory scan
        let result = verify_agent_file_with_strategy("dev", dir.path(), None);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Strategy persona discovery tests (Task 6.4)
    // =========================================================================

    #[test]
    fn test_discover_strategy_personas() {
        let dir = TempDir::new().unwrap();
        make_temp_agent(&dir, "architect", "System architect");
        make_temp_agent(&dir, "developer", "Code writer");

        let strategy = make_strategy(&[
            ("architect", ".claude/agents/architect.md"),
            ("developer", ".claude/agents/developer.md"),
        ]);

        let personas = discover_strategy_personas(dir.path(), &strategy);
        assert_eq!(personas.len(), 2);
        // Sorted alphabetically
        assert_eq!(personas[0].name, "architect");
        assert_eq!(personas[1].name, "developer");
        // All tagged as Strategy source
        for p in &personas {
            assert_eq!(p.source, PersonaSource::Strategy);
        }
    }

    #[test]
    fn test_discover_strategy_personas_missing_file() {
        let dir = TempDir::new().unwrap();
        // Don't create any agent files

        let strategy = make_strategy(&[("ghost", ".claude/agents/ghost.md")]);

        let personas = discover_strategy_personas(dir.path(), &strategy);
        assert!(
            personas.is_empty(),
            "Missing files should be silently skipped"
        );
    }

    #[test]
    fn test_discover_with_strategy_none_uses_directories() {
        let dir = TempDir::new().unwrap();
        make_temp_agent(&dir, "dev", "Dev");

        let personas = discover_personas_with_strategy(dir.path(), None);
        // Should find at least the project persona; may also find user-level agents
        let dev = personas.iter().find(|p| p.name == "dev");
        assert!(dev.is_some(), "Should find the project-local 'dev' persona");
        assert_eq!(dev.unwrap().source, PersonaSource::Project);
    }

    #[test]
    fn test_discover_strategy_uses_key_as_name() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".claude").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        // Frontmatter says "pm" but strategy key says "product-manager"
        std::fs::write(
            agents_dir.join("product-manager.md"),
            "---\nname: pm\ndescription: Product manager\n---\n",
        )
        .unwrap();

        let strategy = make_strategy(&[("product-manager", ".claude/agents/product-manager.md")]);

        let personas = discover_strategy_personas(dir.path(), &strategy);
        assert_eq!(personas.len(), 1);
        assert_eq!(
            personas[0].name, "product-manager",
            "Should use strategy key, not frontmatter name"
        );
    }
}
