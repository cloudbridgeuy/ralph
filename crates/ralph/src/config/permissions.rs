use crate::config::AppConfig;
use crate::warn::warn;
use ralph_core::worker::PermissionMode;

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
