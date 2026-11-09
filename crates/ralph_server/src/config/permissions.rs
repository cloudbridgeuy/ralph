use crate::config::AppConfig;
use crate::warn::warn;
use ralph_core::worker::PermissionMode;

pub const CHAT_DEFAULT_PERMISSION_MODE: &str = "auto";
pub const ASK_DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";

pub fn resolve_permission_mode(
    flag: Option<&str>,
    env: Option<&str>,
    config: Option<&str>,
    default: &str,
) -> String {
    flag.or(env.filter(|value| !value.is_empty()))
        .or(config)
        .unwrap_or(default)
        .to_string()
}

pub fn permission_setting(
    section: &str,
    value: Option<&toml::Value>,
) -> (PermissionMode, Option<String>) {
    match value.map(toml::Value::as_str) {
        None | Some(Some("ask")) => (PermissionMode::Ask, None),
        Some(Some("yolo")) => (PermissionMode::Yolo, None),
        Some(_) => (
            PermissionMode::Ask,
            Some(format!(
                "[{section}] permissions must be \"ask\" or \"yolo\"; using \"ask\""
            )),
        ),
    }
}

pub fn configured_permission_mode(
    harness: &str,
    pick: fn(&AppConfig) -> (PermissionMode, Option<String>),
) -> PermissionMode {
    match AppConfig::load() {
        Ok(config) => {
            let (mode, warning) = pick(&config);
            if let Some(warning) = warning {
                warn(warning);
            }
            mode
        }
        Err(e) => {
            warn(format!(
                "could not read config: {e}; {harness} permissions use \"ask\""
            ));
            PermissionMode::Ask
        }
    }
}
