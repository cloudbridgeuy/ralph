use super::permissions::permission_setting;
use ralph_core::worker::{ClaudeShell, PermissionMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClaudeSection {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub shell: Option<toml::Value>,
    #[serde(default)]
    pub config_dir: Option<PathBuf>,
    #[serde(default)]
    pub permissions: Option<toml::Value>,
}

impl ClaudeSection {
    pub fn shell_setting(&self) -> (ClaudeShell, Option<String>) {
        match &self.shell {
            None => (ClaudeShell::Zsh, None),
            Some(value) => match value.clone().try_into::<ClaudeShell>() {
                Ok(shell) => (shell, None),
                Err(_) => (
                    ClaudeShell::Zsh,
                    Some("[claude] shell must be \"zsh\" or \"bash\"; using \"zsh\"".to_string()),
                ),
            },
        }
    }

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
    fn claude_shell_parse_default_to_zsh_and_warn_on_wrong_values() {
        let parse = |text: &str| {
            toml::from_str::<AppConfig>(text)
                .unwrap()
                .claude
                .shell_setting()
        };
        for bad in ["\"fish\"", "1", "[\"bash\"]"] {
            let (shell, warning) = parse(&format!("[claude]\nshell = {bad}\n"));
            assert_eq!(shell, ClaudeShell::Zsh);
            assert_eq!(
                warning.as_deref(),
                Some("[claude] shell must be \"zsh\" or \"bash\"; using \"zsh\"")
            );
        }
        assert_eq!(parse(""), (ClaudeShell::Zsh, None));
        assert_eq!(
            parse("[claude]\nshell = \"bash\"\n"),
            (ClaudeShell::Bash, None)
        );
        assert_eq!(
            parse("[claude]\nshell = \"zsh\"\n"),
            (ClaudeShell::Zsh, None)
        );
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
