use ralph_core::worker::ClaudeShell;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaudeSection {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub shell: ClaudeShell,
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
}
