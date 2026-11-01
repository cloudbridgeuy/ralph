use super::view::{env_settings, merged_rows};
use super::{
    config_path, hard_errors, leaf_sources, missing_config_dir, read_layer, AppConfig, ConfigError,
};
use std::path::{Path, PathBuf};
use toml::Table;

type CommandResult<T> = Result<T, Box<dyn std::error::Error>>;

fn layer_config(path: &Path) -> Option<AppConfig> {
    read_layer(path)
        .ok()
        .and_then(|table| table.try_into::<AppConfig>().ok())
}

fn prompt_file_errors(path: &Path) -> Vec<String> {
    let Some(config) = layer_config(path) else {
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
    let config_dir_note = layer_config(path)
        .and_then(|config| missing_config_dir(&config, dirs::home_dir().as_deref(), Path::exists));
    report
        .notes
        .iter()
        .chain(&config_dir_note)
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
    let rows = leaf_sources(&effective, &global_layer, &store_layer);
    let env = env_settings(&[], |name| std::env::var(name).ok());
    merged_rows(&rows, &env)
        .iter()
        .for_each(|row| println!("{row}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
