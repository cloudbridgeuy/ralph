//! Persona command — thin handler delegating to the shared invocation engine.
//!
//! Verifies the agent file exists, builds an `InvocationConfig` with
//! `agent: Some(name)`, and calls `invoke()`.

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
