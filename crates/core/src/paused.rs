//! Paused state for hard-stopped sessions.
//!
//! When a user presses 'S' (hard stop) during strategy execution, the session
//! state is saved to allow later resumption with `ralph strategy execute <name> --resume`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Paused state written when a session is hard-stopped.
///
/// This allows resuming the session later with `ralph strategy execute <name> --resume`.
/// Stored at `{data_dir}/paused.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PausedState {
    /// Session slug that was paused
    pub slug: String,
    /// Absolute path to the project directory
    pub project: PathBuf,
    /// Number of iterations completed before the hard stop
    pub iterations_completed: u32,
    /// PRD file path used for this session
    pub prd_path: PathBuf,
    /// When the session was paused
    pub paused_at: DateTime<Utc>,
}

impl PausedState {
    /// Create a new paused state.
    pub fn new(
        slug: String,
        project: PathBuf,
        iterations_completed: u32,
        prd_path: PathBuf,
    ) -> Self {
        Self {
            slug,
            project,
            iterations_completed,
            prd_path,
            paused_at: Utc::now(),
        }
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Deserialize from TOML string.
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paused_state_new() {
        let state = PausedState::new(
            "test-slug".to_string(),
            PathBuf::from("/test/project"),
            5,
            PathBuf::from("/test/prd.toml"),
        );
        assert_eq!(state.slug, "test-slug");
        assert_eq!(state.project, PathBuf::from("/test/project"));
        assert_eq!(state.iterations_completed, 5);
        assert_eq!(state.prd_path, PathBuf::from("/test/prd.toml"));
    }

    #[test]
    fn test_paused_state_toml_roundtrip() {
        let state = PausedState::new(
            "brave-panda".to_string(),
            PathBuf::from("/project"),
            3,
            PathBuf::from("/project/prd.toml"),
        );
        let toml_str = state.to_toml().unwrap();
        let parsed = PausedState::from_toml(&toml_str).unwrap();

        assert_eq!(parsed.slug, state.slug);
        assert_eq!(parsed.project, state.project);
        assert_eq!(parsed.iterations_completed, state.iterations_completed);
        assert_eq!(parsed.prd_path, state.prd_path);
    }
}
