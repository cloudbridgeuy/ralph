use super::permissions::permission_setting;
use ralph_core::worker::{ClaudeShell, PermissionMode};
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
    #[serde(default)]
    pub permissions: Option<toml::Value>,
}

impl ClaudeSection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("claude", self.permissions.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn claude_permissions_parse_default_to_ask_and_warn_on_wrong_values() {
        let parse = |text: &str| {
            toml::from_str::<AppConfig>(text)
                .unwrap()
                .claude
                .permissions_setting()
        };
        assert_eq!(
            parse("[claude]\npermissions = \"yolo\"\n"),
            (PermissionMode::Yolo, None)
        );
        assert_eq!(parse(""), (PermissionMode::Ask, None));
        let (mode, warning) = parse("[claude]\npermissions = 1\n");
        assert_eq!(mode, PermissionMode::Ask);
        assert!(warning.unwrap().starts_with("[claude]"));
    }

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
