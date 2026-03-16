//! Persona discovery and metadata.
//!
//! Pure functions for parsing persona frontmatter from agent files
//! and deduplicating personas from multiple discovery locations.

use std::collections::HashMap;

/// Where a persona was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonaSource {
    /// Resolved from `.claude/strategy.toml` [agents] table.
    Strategy,
    /// Project-local `.claude/agents/` directory.
    Project,
    /// User-level `~/.claude/agents/` directory.
    User,
}

impl PersonaSource {
    /// Human-readable label for display.
    pub fn as_str(&self) -> &'static str {
        match self {
            PersonaSource::Strategy => "strategy",
            PersonaSource::Project => "project",
            PersonaSource::User => "user",
        }
    }
}

/// Metadata extracted from a persona agent file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaInfo {
    pub name: String,
    pub description: String,
    pub source: PersonaSource,
}

/// Parse persona frontmatter from agent file contents.
///
/// Extracts `name` and `description` from YAML-style frontmatter delimited
/// by `---` lines. Returns `None` if frontmatter is missing, malformed, or
/// lacks required fields (`name` and `description`).
///
/// Does **not** use `serde_yaml` — manually splits on `key: value` lines.
pub fn parse_persona_frontmatter(content: &str, source: PersonaSource) -> Option<PersonaInfo> {
    let mut lines = content.lines();

    // First line must be `---`
    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut found_closing = false;

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            found_closing = true;
            break;
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "name" => name = Some(value.to_string()),
                "description" => description = Some(value.to_string()),
                _ => {} // ignore unknown keys like tools, permissionMode
            }
        }
    }

    if !found_closing {
        return None;
    }

    Some(PersonaInfo {
        name: name?,
        description: description?,
        source,
    })
}

/// Deduplicate personas from project and user directories.
///
/// Project personas take precedence: if a persona with the same name exists
/// in both project and user directories, the project version wins.
/// Results are sorted alphabetically by name.
pub fn deduplicate_personas(project: Vec<PersonaInfo>, user: Vec<PersonaInfo>) -> Vec<PersonaInfo> {
    let mut map: HashMap<String, PersonaInfo> = HashMap::new();

    // Insert project personas first (they take precedence)
    for p in project {
        map.insert(p.name.clone(), p);
    }

    // Insert user personas only if not already present
    for u in user {
        map.entry(u.name.clone()).or_insert(u);
    }

    let mut result: Vec<PersonaInfo> = map.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Frontmatter parser tests
    // =========================================================================

    #[test]
    fn test_parse_valid_frontmatter() {
        let content = "\
---
name: dev
description: A development assistant
tools: Read, Edit, Bash
permissionMode: bypassPermissions
---

System prompt here.
";
        let info = parse_persona_frontmatter(content, PersonaSource::Project).unwrap();
        assert_eq!(info.name, "dev");
        assert_eq!(info.description, "A development assistant");
        assert_eq!(info.source, PersonaSource::Project);
    }

    #[test]
    fn test_parse_name_and_description_only() {
        let content = "\
---
name: reviewer
description: Code review persona
---

You are a code reviewer.
";
        let info = parse_persona_frontmatter(content, PersonaSource::User).unwrap();
        assert_eq!(info.name, "reviewer");
        assert_eq!(info.description, "Code review persona");
        assert_eq!(info.source, PersonaSource::User);
    }

    #[test]
    fn test_parse_missing_name() {
        let content = "\
---
description: No name field
---
";
        assert!(parse_persona_frontmatter(content, PersonaSource::Project).is_none());
    }

    #[test]
    fn test_parse_missing_description() {
        let content = "\
---
name: incomplete
---
";
        assert!(parse_persona_frontmatter(content, PersonaSource::Project).is_none());
    }

    #[test]
    fn test_parse_no_delimiters() {
        let content = "Just plain text without frontmatter.";
        assert!(parse_persona_frontmatter(content, PersonaSource::Project).is_none());
    }

    #[test]
    fn test_parse_unclosed_frontmatter() {
        let content = "\
---
name: broken
description: Never closed
";
        assert!(parse_persona_frontmatter(content, PersonaSource::Project).is_none());
    }

    #[test]
    fn test_parse_empty_content() {
        assert!(parse_persona_frontmatter("", PersonaSource::Project).is_none());
    }

    #[test]
    fn test_parse_trims_whitespace() {
        let content = "\
---
name:   spaced-out
description:   Has extra spaces
---
";
        let info = parse_persona_frontmatter(content, PersonaSource::Project).unwrap();
        assert_eq!(info.name, "spaced-out");
        assert_eq!(info.description, "Has extra spaces");
    }

    // =========================================================================
    // Deduplication tests
    // =========================================================================

    #[test]
    fn test_dedup_no_overlap() {
        let project = vec![
            PersonaInfo {
                name: "alpha".to_string(),
                description: "Project alpha".to_string(),
                source: PersonaSource::Project,
            },
            PersonaInfo {
                name: "beta".to_string(),
                description: "Project beta".to_string(),
                source: PersonaSource::Project,
            },
        ];
        let user = vec![
            PersonaInfo {
                name: "gamma".to_string(),
                description: "User gamma".to_string(),
                source: PersonaSource::User,
            },
            PersonaInfo {
                name: "delta".to_string(),
                description: "User delta".to_string(),
                source: PersonaSource::User,
            },
        ];
        let result = deduplicate_personas(project, user);
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].name, "alpha");
        assert_eq!(result[1].name, "beta");
        assert_eq!(result[2].name, "delta");
        assert_eq!(result[3].name, "gamma");
    }

    #[test]
    fn test_dedup_project_shadows_user() {
        let project = vec![PersonaInfo {
            name: "dev".to_string(),
            description: "Project dev".to_string(),
            source: PersonaSource::Project,
        }];
        let user = vec![PersonaInfo {
            name: "dev".to_string(),
            description: "User dev".to_string(),
            source: PersonaSource::User,
        }];
        let result = deduplicate_personas(project, user);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].description, "Project dev");
        assert_eq!(result[0].source, PersonaSource::Project);
    }

    #[test]
    fn test_dedup_mixed() {
        let project = vec![
            PersonaInfo {
                name: "dev".to_string(),
                description: "Project dev".to_string(),
                source: PersonaSource::Project,
            },
            PersonaInfo {
                name: "unique-project".to_string(),
                description: "Only in project".to_string(),
                source: PersonaSource::Project,
            },
        ];
        let user = vec![
            PersonaInfo {
                name: "dev".to_string(),
                description: "User dev".to_string(),
                source: PersonaSource::User,
            },
            PersonaInfo {
                name: "unique-user".to_string(),
                description: "Only in user".to_string(),
                source: PersonaSource::User,
            },
        ];
        let result = deduplicate_personas(project, user);
        assert_eq!(result.len(), 3);
        // Sorted: dev, unique-project, unique-user
        assert_eq!(result[0].name, "dev");
        assert_eq!(result[0].source, PersonaSource::Project);
        assert_eq!(result[1].name, "unique-project");
        assert_eq!(result[2].name, "unique-user");
    }

    #[test]
    fn test_dedup_empty_project() {
        let user = vec![PersonaInfo {
            name: "helper".to_string(),
            description: "User helper".to_string(),
            source: PersonaSource::User,
        }];
        let result = deduplicate_personas(vec![], user);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "helper");
    }

    #[test]
    fn test_dedup_empty_user() {
        let project = vec![PersonaInfo {
            name: "helper".to_string(),
            description: "Project helper".to_string(),
            source: PersonaSource::Project,
        }];
        let result = deduplicate_personas(project, vec![]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "helper");
    }

    #[test]
    fn test_dedup_both_empty() {
        let result = deduplicate_personas(vec![], vec![]);
        assert!(result.is_empty());
    }
}
