use crate::config::{AppConfig, ChatSection};
use crate::warn::warn;
use ralph_core::chat_thread::config_root;
use ralph_core::worker::{answer_message, merged_grants, parse_grants, Choice, Classes, Grants};
use ralph_core::worker::{ClaudeMode, PermissionMode, UnknownClaudeMode};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub const GRANTS_FILE: &str = "mcptools-grants.json";
pub const CHAT_DEFAULT_PERMISSION_MODE: ClaudeMode = ClaudeMode::Auto;
pub const ASK_DEFAULT_PERMISSION_MODE: ClaudeMode = ClaudeMode::BypassPermissions;

#[derive(Debug, thiserror::Error)]
#[error("{origin}: {error}")]
pub struct PermissionModeError {
    origin: &'static str,
    error: UnknownClaudeMode,
}

pub fn resolve_permission_mode(
    flag: Option<&str>,
    (env_name, env): (&'static str, Option<&str>),
    (config_key, config): (&'static str, Option<&str>),
    default: ClaudeMode,
) -> Result<ClaudeMode, PermissionModeError> {
    flag.map(|value| ("--permission-mode", value))
        .or(env
            .filter(|value| !value.is_empty())
            .map(|value| (env_name, value)))
        .or(config.map(|value| (config_key, value)))
        .map_or(Ok(default), |(origin, value)| {
            value
                .parse()
                .map_err(|error| PermissionModeError { origin, error })
        })
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

pub fn saved_grants(store_dir: &Path) -> Grants {
    let text = read_grants_text(&config_root(store_dir).join(GRANTS_FILE));
    let (saved, warning) = parse_grants(text.as_deref());
    warning.into_iter().for_each(warn);
    saved
}

pub fn save_grants(store_dir: &Path, classes: Classes) -> Result<PathBuf, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_grants_read_only_the_file_and_default_to_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert_eq!(saved_grants(dir.path()), Grants::default());
        save_grants(
            dir.path(),
            Classes {
                writes: true,
                spend: false,
            },
        )
        .unwrap();
        assert_eq!(
            saved_grants(dir.path()),
            Grants {
                writes: true,
                spend: false
            }
        );
    }
}
