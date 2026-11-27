use crate::chat::skills::SKILLS_ENV;
use crate::invoke::{ASK_PERMISSION_MODE_ENV, CHAT_PERMISSION_MODE_ENV};
use ralph_core::worker::WORKTREE_DIR_ENV;
use std::fmt;
use toml::Value;

pub const WRAP_WIDTH_ENV: &str = "RALPH_WRAP_WIDTH";
pub const CELL_OUTPUT_LIMIT_ENV: &str = "RALPH_CELL_OUTPUT_LIMIT";
pub const CELL_OUTPUT_BYTES_ENV: &str = "RALPH_CELL_OUTPUT_BYTES";
pub const RALPH_THEME_ENV: &str = "RALPH_THEME";
pub const RALPH_NO_BACKGROUND_ENV: &str = "RALPH_NO_BACKGROUND";

const ENV_KEYS: [(&str, &str); 9] = [
    ("ask.claude.permission_mode", ASK_PERMISSION_MODE_ENV),
    ("chat.claude.permission_mode", CHAT_PERMISSION_MODE_ENV),
    ("chat.wrap_width", WRAP_WIDTH_ENV),
    ("chat.cell_output_limit", CELL_OUTPUT_LIMIT_ENV),
    ("chat.cell_output_bytes", CELL_OUTPUT_BYTES_ENV),
    ("chat.skills", SKILLS_ENV),
    ("chat.worktree_dir", WORKTREE_DIR_ENV),
    ("theme.name", RALPH_THEME_ENV),
    ("theme.no_background", RALPH_NO_BACKGROUND_ENV),
];

const EXTRA_OVERRIDES: [(&str, &str); 1] = [("theme.assistant.code", RALPH_THEME_ENV)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Env(&'static str),
    Flag(&'static str),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Env(name) => write!(f, "env {name}"),
            Origin::Flag(name) => write!(f, "flag {name}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    pub origin: Origin,
    pub value: Value,
    pub key: Option<&'static str>,
}

pub fn env_settings(
    extra: &[&'static str],
    lookup: impl Fn(&str) -> Option<String>,
) -> Vec<Setting> {
    let keyed = ENV_KEYS.iter().map(|(key, name)| (*name, Some(*key)));
    let unkeyed = extra.iter().map(|name| (*name, None));
    keyed
        .chain(unkeyed)
        .filter_map(|(name, key)| {
            lookup(name)
                .filter(|value| !value.is_empty())
                .map(|value| Setting {
                    origin: Origin::Env(name),
                    value: Value::String(value),
                    key,
                })
        })
        .collect()
}

pub fn winning_override<'a>(overrides: &'a [Setting], key: &str) -> Option<&'a Setting> {
    overrides.iter().find(|setting| {
        setting.key == Some(key)
            || EXTRA_OVERRIDES
                .iter()
                .any(|(extra, name)| *extra == key && setting.origin == Origin::Env(name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &'static str, value: &str, key: Option<&'static str>) -> Setting {
        Setting {
            origin: Origin::Env(name),
            value: Value::String(value.into()),
            key,
        }
    }

    #[test]
    fn env_settings_keep_set_known_names_only() {
        let lookup = |name: &str| match name {
            "RALPH_WRAP_WIDTH" => Some("100".to_string()),
            "RALPH_THEME" => Some(String::new()),
            "NO_COLOR" => Some("1".to_string()),
            "OTHER" => Some("x".to_string()),
            _ => None,
        };
        assert_eq!(
            env_settings(&["NO_COLOR"], lookup),
            [
                env("RALPH_WRAP_WIDTH", "100", Some("chat.wrap_width")),
                env("NO_COLOR", "1", None),
            ]
        );
    }

    #[test]
    fn a_keyed_setting_wins_its_own_key() {
        let overrides = [env(WRAP_WIDTH_ENV, "100", Some("chat.wrap_width"))];
        assert_eq!(
            winning_override(&overrides, "chat.wrap_width"),
            overrides.first()
        );
        assert_eq!(winning_override(&overrides, "chat.other"), None);
    }

    #[test]
    fn the_theme_env_also_wins_the_assistant_code_key() {
        let overrides = [env(RALPH_THEME_ENV, "Y", Some("theme.name"))];
        assert_eq!(
            winning_override(&overrides, "theme.assistant.code"),
            overrides.first()
        );
    }
}
