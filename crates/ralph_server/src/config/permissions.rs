use crate::config::{AppConfig, ChatSection};
use crate::warn::warn;
use ralph_core::chat_thread::config_root;
use ralph_core::worker::PermissionMode;
use ralph_core::worker::{answer_message, merged_grants, parse_grants, Choice, Classes, Grants};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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

fn read_grants_text(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => {
            warn(format!(
                "could not read {}: {e}; no saved grants",
                path.display()
            ));
            None
        }
    }
}

pub fn effective_grants(chat: &ChatSection, store_dir: &Path) -> Grants {
    let (mode, mode_warning) = chat.permissions_setting();
    let text = read_grants_text(&config_root(store_dir).join(GRANTS_FILE));
    let (saved, saved_warning) = parse_grants(text.as_deref());
    mode_warning.into_iter().chain(saved_warning).for_each(warn);
    Grants::for_mode(mode).union(saved)
}

fn save_grants(store_dir: &Path, classes: Classes) -> Result<PathBuf, String> {
    let path = config_root(store_dir).join(GRANTS_FILE);
    let (json, warning) = merged_grants(read_grants_text(&path).as_deref(), classes);
    warning.into_iter().for_each(warn);
    let write_error = |e: std::io::Error| {
        format!(
            "could not save {}: {e}; the grant lasts this session only",
            path.display()
        )
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(write_error)?;
    }
    fs::write(&path, json).map_err(write_error)?;
    Ok(path)
}

pub fn confirm_answer(store_dir: &Path, choice: Choice, classes: Classes) -> String {
    let saved = (choice == Choice::AllowAlways)
        .then(|| save_grants(store_dir, classes))
        .and_then(|saved| saved.map_err(warn).ok());
    let shown = saved.map(|path| path.display().to_string());
    answer_message(choice, classes, shown.as_deref())
}
