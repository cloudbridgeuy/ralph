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
}

/// Check that the agent file exists in one of claude's agent directories.
///
/// Search order:
/// 1. Project-local: `.claude/agents/{name}.md`
/// 2. User-level: `~/.claude/agents/{name}.md`
///
/// Returns the resolved path on success.
pub fn verify_agent_file(persona_name: &str, project_path: &Path) -> Result<PathBuf, PersonaError> {
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

    let source_width = PersonaSource::Project.as_str().len(); // longest source label

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
}
