use crate::config::{AppConfig, ChatSection};
use crate::warn::warn;
use ralph_core::chat_thread::config_root;
use ralph_core::worker::PermissionMode;
use ralph_core::worker::{parse_grants, Grants};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;

pub const GRANTS_FILE: &str = "mcptools-grants.json";
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

pub fn effective_grants(chat: &ChatSection, store_dir: &Path) -> Grants {
    let (mode, mode_warning) = chat.permissions_setting();
    let path = config_root(store_dir).join(GRANTS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => {
            warn(format!(
                "could not read {}: {e}; no saved grants",
                path.display()
            ));
            None
        }
    };
    let (saved, saved_warning) = parse_grants(text.as_deref());
    mode_warning.into_iter().chain(saved_warning).for_each(warn);
    Grants::for_mode(mode).union(saved)
}
