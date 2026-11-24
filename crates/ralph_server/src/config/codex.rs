use super::permissions::permission_setting;
use super::AppConfig;
use crate::warn::warn;
use ralph_core::worker::{codex_config_warning, PermissionMode, SandboxMode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CodexSection {
    #[serde(default)]
    pub permissions: Option<toml::Value>,
    #[serde(default)]
    pub sandbox: Option<toml::Value>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl CodexSection {
    pub fn permissions_setting(&self) -> (PermissionMode, Option<String>) {
        permission_setting("codex", self.permissions.as_ref())
    }

    pub fn sandbox_mode(&self) -> Result<Option<SandboxMode>, String> {
        self.sandbox
            .as_ref()
            .map(|value| {
                value
                    .as_str()
                    .and_then(SandboxMode::parse)
                    .ok_or_else(|| {
                        "[codex] sandbox must be \"read-only\", \"workspace-write\" or \"danger-full-access\""
                            .to_string()
                    })
            })
            .transpose()
    }
}

pub fn configured_codex() -> CodexSection {
    AppConfig::load()
        .map(|config| config.codex)
        .unwrap_or_else(|e| {
            if let Some(text) = codex_config_warning(Err(&e.to_string())) {
                warn(text);
            }
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

    #[test]
    fn codex_sandbox_parses_each_mode_and_rejects_anything_else() {
        let parse = |text: &str| {
            toml::from_str::<AppConfig>(text)
                .unwrap()
                .codex
                .sandbox_mode()
        };
        assert_eq!(parse(""), Ok(None));
        for (text, mode) in [
            ("read-only", SandboxMode::ReadOnly),
            ("workspace-write", SandboxMode::WorkspaceWrite),
            ("danger-full-access", SandboxMode::DangerFullAccess),
        ] {
            assert_eq!(
                parse(&format!("[codex]\nsandbox = \"{text}\"\n")),
                Ok(Some(mode))
            );
        }
        for bad in ["\"nope\"", "true", "1"] {
            let error = parse(&format!("[codex]\nsandbox = {bad}\n")).unwrap_err();
            assert!(error.starts_with("[codex]"), "{error}");
        }
    }
}
