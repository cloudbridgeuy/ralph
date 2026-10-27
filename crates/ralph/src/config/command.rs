use super::{config_path, hard_errors, leaf_sources, read_layer, AppConfig, ConfigError, Source};
use crate::chat::{CELL_OUTPUT_BYTES_ENV, CELL_OUTPUT_LIMIT_ENV, SKILLS_ENV, WRAP_WIDTH_ENV};
use crate::highlight::{RALPH_NO_BACKGROUND_ENV, RALPH_THEME_ENV};
use crate::invoke::{ASK_PERMISSION_MODE_ENV, CHAT_PERMISSION_MODE_ENV};
use ralph_core::worker::WORKTREE_DIR_ENV;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

type CommandResult<T> = Result<T, Box<dyn std::error::Error>>;

const ENV_KEYS: [(&str, &str); 9] = [
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

fn set_env_name(key: &str, is_set: impl Fn(&str) -> bool) -> Option<&'static str> {
    ENV_KEYS
        .iter()
        .find(|(candidate, name)| *candidate == key && is_set(name))
        .map(|(_, name)| *name)
}

fn show_row(key: &str, value: &Value, source: Source, env: Option<&str>) -> String {
    match env {
        Some(name) => format!("{key} = {value}  [{}; env {name} is set]", source.label()),
        None => format!("{key} = {value}  [{}]", source.label()),
    }
}

fn prompt_file_errors(path: &Path) -> Vec<String> {
    let Some(config) = read_layer(path)
        .ok()
        .and_then(|table| table.try_into::<AppConfig>().ok())
    else {
        return Vec::new();
    };
    let files = config.chat.prompt_files(dirs::home_dir().as_deref());
    files
        .behaviour
        .iter()
        .chain(&files.appends)
        .filter_map(|file| {
            std::fs::read_to_string(file)
                .err()
                .map(|e| format!("cannot read prompt file {}: {e}", file.display()))
        })
        .collect()
}

fn check_file(path: &Path, required: bool, prefixed: bool) -> CommandResult<Vec<String>> {
    let label = path.display().to_string();
    if !path.exists() && !required {
        println!("no config at {label}; defaults apply");
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path).map_err(|e| format!("cannot read {label}: {e}"))?;
    let report = super::check(&content, &label)?;
    let line = |text: &str| {
        if prefixed {
            format!("{label}: {text}")
        } else {
            text.to_string()
        }
    };
    let errors = [report.errors, prompt_file_errors(path)].concat();
    report
        .notes
        .iter()
        .for_each(|note| eprintln!("note: {}", line(note)));
    errors.iter().for_each(|error| eprintln!("{}", line(error)));
    if errors.is_empty() {
        println!("ok: {label}");
    }
    Ok(errors)
}

pub fn run_check(path: Option<PathBuf>, store: Option<PathBuf>) -> CommandResult<()> {
    let required = path.is_some();
    let global = path.unwrap_or_else(config_path);
    let Some(store) = store else {
        let errors = check_file(&global, required, false)?;
        return match errors.len() {
            0 => Ok(()),
            n => Err(format!("{n} problem(s) in {}", global.display()).into()),
        };
    };
    let store_file = store.join("config.toml");
    let results = [
        check_file(&global, required, true),
        check_file(&store_file, false, true),
    ];
    let mut problems = 0;
    let mut seen = Vec::new();
    let mut readable = true;
    for result in results {
        match result {
            Ok(errors) => {
                problems += errors.len();
                seen.extend(errors);
            }
            Err(e) => {
                eprintln!("{e}");
                problems += 1;
                readable = false;
            }
        }
    }
    if readable {
        match AppConfig::load_layers(&global, Some(&store_file)) {
            Ok(config) => {
                for error in hard_errors(&config)
                    .into_iter()
                    .filter(|e| !seen.contains(e))
                {
                    eprintln!("merged config: {error}");
                    problems += 1;
                }
            }
            Err(e) => {
                eprintln!("merged config: {e}");
                problems += 1;
            }
        }
    }
    match problems {
        0 => Ok(()),
        n => Err(format!("{n} problem(s)").into()),
    }
}

pub fn run_show(store: Option<PathBuf>) -> CommandResult<()> {
    let global = config_path();
    let store_file = store.map(|dir| dir.join("config.toml"));
    let config = AppConfig::load_layers(&global, store_file.as_deref())?;
    let effective = Table::try_from(&config).map_err(ConfigError::from)?;
    let global_layer = read_layer(&global)?;
    let store_layer = store_file.as_deref().map_or(Ok(Table::new()), read_layer)?;
    println!("global: {}", global.display());
    if let Some(file) = &store_file {
        println!("store: {}", file.display());
    }
    for (key, value, source) in leaf_sources(&effective, &global_layer, &store_layer) {
        let env = set_env_name(&key, |name| {
            std::env::var_os(name).is_some_and(|v| !v.is_empty())
        });
        println!("{}", show_row(&key, &value, source, env));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_marker_follows_the_key_and_the_set_check() {
        assert_eq!(
            set_env_name("chat.wrap_width", |name| name == WRAP_WIDTH_ENV),
            Some(WRAP_WIDTH_ENV)
        );
        assert_eq!(set_env_name("chat.wrap_width", |_| false), None);
        assert_eq!(set_env_name("chat.worker_base", |_| true), None);
    }

    #[test]
    fn show_row_lists_source_and_env() {
        let value = Value::Integer(80);
        assert_eq!(
            show_row("chat.wrap_width", &value, Source::Store, None),
            "chat.wrap_width = 80  [store]"
        );
        assert_eq!(
            show_row("chat.wrap_width", &value, Source::Global, Some("X")),
            "chat.wrap_width = 80  [global; env X is set]"
        );
    }

    #[test]
    fn prompt_file_errors_name_missing_files_relative_to_the_config() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("here.md"), "x").unwrap();
        let config = dir.path().join("config.toml");
        std::fs::write(
            &config,
            "[chat]\nprompt_file = \"here.md\"\nappend_prompt_files = [\"gone.md\"]\n",
        )
        .unwrap();
        let errors = prompt_file_errors(&config);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains(&dir.path().join("gone.md").display().to_string()));
        assert!(prompt_file_errors(&dir.path().join("none.toml")).is_empty());
    }
}
