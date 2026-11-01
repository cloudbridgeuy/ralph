use super::{leaf_sources, read_layer, AppConfig, Notice, PromptFiles, Source};
use crate::chat::{CELL_OUTPUT_BYTES_ENV, CELL_OUTPUT_LIMIT_ENV, SKILLS_ENV, WRAP_WIDTH_ENV};
use crate::highlight::{RALPH_NO_BACKGROUND_ENV, RALPH_THEME_ENV};
use crate::invoke::{ASK_PERMISSION_MODE_ENV, CHAT_PERMISSION_MODE_ENV};
use crate::warn::{warn, warn_once};
use ralph_core::worker::WORKTREE_DIR_ENV;
use std::fmt;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

pub(super) const ENV_KEYS: [(&str, &str); 9] = [
    ("ask.permission_mode", ASK_PERMISSION_MODE_ENV),
    ("chat.permission_mode", CHAT_PERMISSION_MODE_ENV),
    ("chat.wrap_width", WRAP_WIDTH_ENV),
    ("chat.cell_output_limit", CELL_OUTPUT_LIMIT_ENV),
    ("chat.cell_output_bytes", CELL_OUTPUT_BYTES_ENV),
    ("chat.skills", SKILLS_ENV),
    ("chat.worktree_dir", WORKTREE_DIR_ENV),
    ("theme.name", RALPH_THEME_ENV),
    ("theme.no_background", RALPH_NO_BACKGROUND_ENV),
];

pub type Row = (String, Value, Source);

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

#[derive(Debug, Clone, PartialEq)]
pub struct FileState {
    pub path: PathBuf,
    pub exists: bool,
}

impl FileState {
    pub fn of(path: PathBuf) -> Self {
        let exists = path.exists();
        Self { path, exists }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub warnings: Vec<String>,
    pub global: FileState,
    pub store_file: FileState,
    pub store_dir: PathBuf,
    pub usage: PathBuf,
    pub claude_dir: Option<PathBuf>,
    pub prompt_files: PromptFiles,
    pub skills: Vec<PathBuf>,
    pub env: Vec<Setting>,
    pub flags: Vec<Setting>,
    pub rows: Vec<Row>,
    pub effective: Option<Table>,
}

pub struct ChatLoad {
    pub config: AppConfig,
    pub warnings: Vec<String>,
    pub rows: Vec<Row>,
    pub effective: Option<Table>,
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

pub fn merged_rows(rows: &[Row], overrides: &[Setting]) -> Vec<String> {
    let winner = |key: &str| overrides.iter().find(|setting| setting.key == Some(key));
    let layered = rows.iter().map(|(key, value, source)| {
        let text = match winner(key) {
            Some(setting) => format!(
                "{key} = {}  [{}; {} = {value}]",
                setting.value,
                setting.origin,
                source.label()
            ),
            None => format!("{key} = {value}  [{}]", source.label()),
        };
        (key.clone(), text)
    });
    let mut added: Vec<(String, String)> = Vec::new();
    for setting in overrides {
        let Some(key) = setting.key else { continue };
        let known = rows.iter().any(|(row, ..)| row == key) || added.iter().any(|(k, _)| k == key);
        if !known {
            let text = format!("{key} = {}  [{}]", setting.value, setting.origin);
            added.push((key.to_string(), text));
        }
    }
    let mut lines: Vec<(String, String)> = layered.chain(added).collect();
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    lines.into_iter().map(|(_, text)| text).collect()
}

const CHANGED_NOTE: &str = "config files changed on disk since start; restart the chat to apply";

fn path_line(label: &str, path: &Path) -> String {
    format!("  {label:<14}{}", path.display())
}

fn file_line(label: &str, file: &FileState) -> String {
    let line = path_line(label, &file.path);
    match file.exists {
        true => line,
        false => format!("{line} (missing)"),
    }
}

fn plain(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Boolean(_) => None,
        other => Some(other.to_string()),
    }
}

fn setting_line(setting: &Setting) -> String {
    let head = match (setting.origin, plain(&setting.value)) {
        (Origin::Env(name), value) => format!("{name}={}", value.unwrap_or_default()),
        (Origin::Flag(name), Some(value)) => format!("{name} {value}"),
        (Origin::Flag(name), None) => name.to_string(),
    };
    match setting.key {
        Some(key) => format!("  {head} (overrides {key})"),
        None => format!("  {head}"),
    }
}

fn settings_block(title: &str, settings: &[Setting], empty: &str) -> Vec<String> {
    let body = match settings.is_empty() {
        true => vec![format!("  {empty}")],
        false => settings.iter().map(setting_line).collect(),
    };
    std::iter::once(title.to_string()).chain(body).collect()
}

fn sources(snapshot: &ConfigSnapshot) -> Vec<String> {
    let files = &snapshot.prompt_files;
    let config_dir = snapshot.global.path.parent();
    let claude = snapshot.claude_dir.as_deref();
    std::iter::once("Sources".to_string())
        .chain(config_dir.map(|dir| path_line("config dir", dir)))
        .chain([
            file_line("global config", &snapshot.global),
            file_line("store config", &snapshot.store_file),
            path_line("store", &snapshot.store_dir),
            path_line("usage record", &snapshot.usage),
        ])
        .chain(claude.map(|dir| path_line("claude dir", dir)))
        .chain(
            files
                .behaviour
                .iter()
                .map(|file| path_line("prompt file", file)),
        )
        .chain(
            files
                .appends
                .iter()
                .map(|file| path_line("append prompt", file)),
        )
        .chain(snapshot.skills.iter().map(|dir| path_line("skill", dir)))
        .collect()
}

pub fn config_view(snapshot: &ConfigSnapshot, changed_on_disk: bool) -> String {
    let overrides: Vec<Setting> = snapshot
        .flags
        .iter()
        .chain(&snapshot.env)
        .cloned()
        .collect();
    let lines: Vec<String> = snapshot
        .warnings
        .iter()
        .map(|warning| format!("warning: {warning}"))
        .chain(changed_on_disk.then(|| CHANGED_NOTE.to_string()))
        .chain(sources(snapshot))
        .chain(settings_block("Environment", &snapshot.env, "none set"))
        .chain(settings_block("Flags", &snapshot.flags, "none"))
        .chain(std::iter::once("Merged config".to_string()))
        .chain(
            merged_rows(&snapshot.rows, &overrides)
                .into_iter()
                .map(|row| format!("  {row}")),
        )
        .collect();
    format!("{}\n", lines.join("\n"))
}

pub fn load_for_chat(global: &Path, store: &Path) -> ChatLoad {
    let (result, notices) = AppConfig::load_from(global, Some(store));
    notices
        .iter()
        .for_each(|notice| warn_once(&notice.key, &notice.message));
    let fell_back = notices.iter().any(Notice::is_store_fallback);
    let mut warnings: Vec<String> = notices.into_iter().map(|notice| notice.message).collect();
    let (config, loaded) = match result {
        Ok(config) => (config, true),
        Err(e) => {
            let message = format!("Failed to load config: {e}");
            warn(&message);
            warnings.push(message);
            (AppConfig::default(), false)
        }
    };
    let table = Table::try_from(&config).unwrap_or_default();
    let global_layer = read_layer(global).unwrap_or_default();
    let store_layer = match fell_back {
        true => Table::new(),
        false => read_layer(store).unwrap_or_default(),
    };
    let rows = leaf_sources(&table, &global_layer, &store_layer);
    ChatLoad {
        config,
        warnings,
        rows,
        effective: loaded.then_some(table),
    }
}

pub fn effective_now(global: &Path, store: &Path) -> Option<Table> {
    let config = AppConfig::load_from(global, Some(store)).0.ok()?;
    Table::try_from(&config).ok()
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

    fn flag(name: &'static str, value: Value, key: Option<&'static str>) -> Setting {
        Setting {
            origin: Origin::Flag(name),
            value,
            key,
        }
    }

    fn row(key: &str, value: Value, source: Source) -> Row {
        (key.to_string(), value, source)
    }

    fn snapshot() -> ConfigSnapshot {
        ConfigSnapshot {
            warnings: Vec::new(),
            global: FileState {
                path: PathBuf::from("/cfg/ralph/config.toml"),
                exists: true,
            },
            store_file: FileState {
                path: PathBuf::from("/s/config.toml"),
                exists: true,
            },
            store_dir: PathBuf::from("/s"),
            usage: PathBuf::from("/cfg/ralph/usage.jsonl"),
            claude_dir: Some(PathBuf::from("/h/.claude")),
            prompt_files: PromptFiles {
                behaviour: Some(PathBuf::from("/cfg/ralph/p.md")),
                appends: vec![PathBuf::from("/s/a.md")],
            },
            skills: vec![PathBuf::from("/h/skills/x")],
            env: Vec::new(),
            flags: Vec::new(),
            rows: vec![row("chat.wrap_width", Value::Integer(80), Source::Global)],
            effective: None,
        }
    }

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn sources_are_listed_with_absolute_paths() {
        let view = config_view(&snapshot(), false);
        let lines = lines(&view);
        for expected in [
            "  config dir    /cfg/ralph",
            "  global config /cfg/ralph/config.toml",
            "  store config  /s/config.toml",
            "  store         /s",
            "  usage record  /cfg/ralph/usage.jsonl",
            "  claude dir    /h/.claude",
            "  prompt file   /cfg/ralph/p.md",
            "  append prompt /s/a.md",
            "  skill         /h/skills/x",
        ] {
            assert!(lines.contains(&expected), "missing {expected:?} in\n{view}");
        }
    }

    #[test]
    fn a_missing_store_file_is_marked() {
        let mut snapshot = snapshot();
        snapshot.store_file.exists = false;
        let view = config_view(&snapshot, false);
        assert!(lines(&view).contains(&"  store config  /s/config.toml (missing)"));
    }

    #[test]
    fn an_env_override_replaces_the_value_and_names_the_layer() {
        let mut snapshot = snapshot();
        snapshot.env = vec![env(WRAP_WIDTH_ENV, "100", Some("chat.wrap_width"))];
        let view = config_view(&snapshot, false);
        let lines = lines(&view);
        assert!(lines.contains(&"  RALPH_WRAP_WIDTH=100 (overrides chat.wrap_width)"));
        assert!(lines.contains(&"  chat.wrap_width = \"100\"  [env RALPH_WRAP_WIDTH; global = 80]"));
    }

    #[test]
    fn a_flag_beats_the_env_for_the_same_key() {
        let mut snapshot = snapshot();
        snapshot.env = vec![env(WRAP_WIDTH_ENV, "100", Some("chat.wrap_width"))];
        snapshot.flags = vec![flag(
            "--wrap-width",
            Value::Integer(90),
            Some("chat.wrap_width"),
        )];
        let view = config_view(&snapshot, false);
        let lines = lines(&view);
        assert!(lines.contains(&"  --wrap-width 90 (overrides chat.wrap_width)"));
        assert!(lines.contains(&"  chat.wrap_width = 90  [flag --wrap-width; global = 80]"));
    }

    #[test]
    fn an_override_of_an_unset_key_adds_a_sorted_row() {
        let rows = [
            row("chat.a", Value::Boolean(true), Source::Default),
            row("chat.z", Value::Boolean(true), Source::Store),
        ];
        let overrides = [flag(
            "--recent-history-tokens",
            Value::Integer(5000),
            Some("chat.recent_history_tokens"),
        )];
        assert_eq!(
            merged_rows(&rows, &overrides),
            [
                "chat.a = true  [default]",
                "chat.recent_history_tokens = 5000  [flag --recent-history-tokens]",
                "chat.z = true  [store]",
            ]
        );
    }

    #[test]
    fn a_flag_without_a_key_shows_in_the_flag_list_only() {
        let mut snapshot = snapshot();
        snapshot.flags = vec![
            flag("--store", Value::String("/s".into()), None),
            flag(
                "--no-inbox-sound",
                Value::Boolean(false),
                Some("chat.inbox_alert_sound"),
            ),
        ];
        let view = config_view(&snapshot, false);
        let lines = lines(&view);
        assert!(lines.contains(&"  --store /s"));
        assert!(lines.contains(&"  --no-inbox-sound (overrides chat.inbox_alert_sound)"));
    }

    #[test]
    fn warnings_and_the_changed_note_come_first() {
        let mut snapshot = snapshot();
        snapshot.warnings = vec!["ignoring store config /s/config.toml: bad".into()];
        let view = config_view(&snapshot, true);
        let lines = lines(&view);
        assert_eq!(
            lines[0],
            "warning: ignoring store config /s/config.toml: bad"
        );
        assert_eq!(
            lines[1],
            "config files changed on disk since start; restart the chat to apply"
        );
        assert!(!config_view(&snapshot, false).contains("changed on disk"));
    }

    #[test]
    fn empty_env_and_flags_say_so() {
        let view = config_view(&snapshot(), false);
        let lines = lines(&view);
        assert!(lines.contains(&"Environment"));
        assert!(lines.contains(&"  none set"));
        assert!(lines.contains(&"Flags"));
        assert!(lines.contains(&"  none"));
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
    fn a_broken_store_file_falls_back_and_its_keys_are_not_store_rows() {
        let dir = tempfile::TempDir::new().unwrap();
        let global = dir.path().join("global.toml");
        let store = dir.path().join("config.toml");
        std::fs::write(&global, "[chat]\nwrap_width = 80\n").unwrap();
        std::fs::write(&store, "[chat]\nwrap_width = \"wide\"\n").unwrap();
        let loaded = load_for_chat(&global, &store);
        assert_eq!(loaded.config.chat.wrap_width, Some(80));
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].starts_with("ignoring store config"));
        let wrap = loaded
            .rows
            .iter()
            .find(|(key, ..)| key == "chat.wrap_width")
            .map(|(_, _, source)| *source);
        assert_eq!(wrap, Some(Source::Global));
        assert_eq!(effective_now(&global, &store), loaded.effective);
        std::fs::write(&global, "[chat]\nwrap_width = 70\n").unwrap();
        assert_ne!(effective_now(&global, &store), loaded.effective);
    }
}
