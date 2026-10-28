use ralph_core::worker::ClaudeShell;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaudeSection {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub shell: ClaudeShell,
    #[serde(default)]
    pub config_dir: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use crate::config::AppConfig;

    #[test]
    fn claude_model_parses_and_defaults_to_unset() {
        let config: AppConfig = toml::from_str("[claude]\nmodel = \"opus\"\n").unwrap();
        assert_eq!(config.claude.model.as_deref(), Some("opus"));
        assert_eq!(AppConfig::new().claude.model, None);
    }

    #[test]
    fn claude_config_dir_parses_and_defaults_to_unset() {
        let config: AppConfig = toml::from_str("[claude]\nconfig_dir = \"~/alt\"\n").unwrap();
        assert_eq!(config.claude.config_dir, Some("~/alt".into()));
        assert_eq!(AppConfig::new().claude.config_dir, None);
    }
}
