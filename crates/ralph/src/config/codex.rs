use super::permissions::permission_setting;
use super::AppConfig;
use crate::warn::warn;
use ralph_core::worker::PermissionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexSection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl CodexSection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("codex", self.permissions.as_ref())
    }
}

pub fn configured_codex() -> CodexSection {
    AppConfig::load()
        .map(|config| config.codex)
        .unwrap_or_else(|e| {
            warn(format!(
                "could not read config: {e}; codex model and effort use codex's own defaults"
            ));
            CodexSection::default()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn codex_model_and_effort_parse_and_default_to_unset() {
        let config: AppConfig =
            toml::from_str("[codex]\nmodel = \"m\"\neffort = \"high\"\n").unwrap();
        assert_eq!(config.codex.model.as_deref(), Some("m"));
        assert_eq!(config.codex.effort.as_deref(), Some("high"));
        assert_eq!(AppConfig::new().codex.effort, None);
    }

    #[test]
    fn codex_permissions_parse_default_to_ask_and_warn_on_wrong_values() {
        let parse = |text: &str| {
            toml::from_str::<AppConfig>(text)
                .unwrap()
                .codex
                .permissions_setting()
        };
        assert_eq!(
            parse("[codex]\npermissions = \"yolo\"\n"),
            (PermissionMode::Yolo, None)
        );
        assert_eq!(parse(""), (PermissionMode::Ask, None));
        let (mode, warning) = parse("[codex]\npermissions = true\n");
        assert_eq!(mode, PermissionMode::Ask);
        assert!(warning.unwrap().starts_with("[codex]"));
    }
}
