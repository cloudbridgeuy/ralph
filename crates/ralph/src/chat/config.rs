use super::STORE_ENV;
use crate::cli::ChatArgs;
use crate::config::{config_view, effective_now, env_settings, ConfigSnapshot, Origin, Setting};
use crate::paths::RALPH_CONFIG_DIR_ENV;
use std::path::Path;
use toml::Value;

const CHAT_ENV: [&str; 5] = [
    RALPH_CONFIG_DIR_ENV,
    STORE_ENV,
    "NO_COLOR",
    "VISUAL",
    "EDITOR",
];

fn path_value(path: &Path) -> Value {
    Value::String(path.display().to_string())
}

fn count_value(count: usize) -> Value {
    Value::Integer(i64::try_from(count).unwrap_or(i64::MAX))
}

pub(super) fn flag_settings(args: &ChatArgs) -> Vec<Setting> {
    let flag = |name: &'static str, value: Value, key: Option<&'static str>| Setting {
        origin: Origin::Flag(name),
        value,
        key,
    };
    let skills = Value::Array(args.skills.iter().map(|dir| path_value(dir)).collect());
    [
        args.store
            .as_deref()
            .map(|dir| flag("--store", path_value(dir), None)),
        args.name
            .clone()
            .map(|name| flag("--name", Value::String(name), Some("chat.name"))),
        args.recent_history_tokens.map(|tokens| {
            flag(
                "--recent-history-tokens",
                count_value(tokens),
                Some("chat.recent_history_tokens"),
            )
        }),
        args.wrap_width
            .map(|width| flag("--wrap-width", count_value(width), Some("chat.wrap_width"))),
        (!args.skills.is_empty()).then(|| flag("--skill", skills, Some("chat.skills"))),
        args.permission_mode.clone().map(|mode| {
            flag(
                "--permission-mode",
                Value::String(mode),
                Some("chat.permission_mode"),
            )
        }),
        args.worktree_dir.clone().map(|dir| {
            flag(
                "--worktree-dir",
                Value::String(dir),
                Some("chat.worktree_dir"),
            )
        }),
        args.no_inbox_sound.then(|| {
            flag(
                "--no-inbox-sound",
                Value::Boolean(false),
                Some("chat.inbox_alert_sound"),
            )
        }),
        args.no_inbox_notification.then(|| {
            flag(
                "--no-inbox-notification",
                Value::Boolean(false),
                Some("chat.inbox_alert_notification"),
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(super) fn chat_env() -> Vec<Setting> {
    env_settings(&CHAT_ENV, |name| std::env::var(name).ok())
}

pub(super) fn print(snapshot: &ConfigSnapshot) {
    let now = effective_now(&snapshot.global.path, &snapshot.store_file.path);
    let changed = now != snapshot.effective;
    print!("{}", config_view(&snapshot.rechecked(), changed));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn args() -> ChatArgs {
        ChatArgs {
            command: None,
            store: None,
            name: None,
            recent_history_tokens: None,
            wrap_width: None,
            skills: Vec::new(),
            permission_mode: None,
            worktree_dir: None,
            no_inbox_sound: false,
            no_inbox_notification: false,
        }
    }

    fn setting(name: &'static str, value: Value, key: Option<&'static str>) -> Setting {
        Setting {
            origin: Origin::Flag(name),
            value,
            key,
        }
    }

    #[test]
    fn no_flags_give_no_settings() {
        assert!(flag_settings(&args()).is_empty());
    }

    #[test]
    fn each_given_flag_names_the_key_it_overrides() {
        let args = ChatArgs {
            command: None,
            store: Some(PathBuf::from("rel/store")),
            name: Some("review".into()),
            recent_history_tokens: Some(5000),
            wrap_width: Some(90),
            skills: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            permission_mode: Some("plan".into()),
            worktree_dir: Some(".wt".into()),
            no_inbox_sound: true,
            no_inbox_notification: true,
        };
        let text = |s: &str| Value::String(s.into());
        assert_eq!(
            flag_settings(&args),
            [
                setting("--store", text("rel/store"), None),
                setting("--name", text("review"), Some("chat.name")),
                setting(
                    "--recent-history-tokens",
                    Value::Integer(5000),
                    Some("chat.recent_history_tokens")
                ),
                setting("--wrap-width", Value::Integer(90), Some("chat.wrap_width")),
                setting(
                    "--skill",
                    Value::Array(vec![text("/a"), text("/b")]),
                    Some("chat.skills")
                ),
                setting(
                    "--permission-mode",
                    text("plan"),
                    Some("chat.permission_mode")
                ),
                setting("--worktree-dir", text(".wt"), Some("chat.worktree_dir")),
                setting(
                    "--no-inbox-sound",
                    Value::Boolean(false),
                    Some("chat.inbox_alert_sound")
                ),
                setting(
                    "--no-inbox-notification",
                    Value::Boolean(false),
                    Some("chat.inbox_alert_notification")
                ),
            ]
        );
    }
}
