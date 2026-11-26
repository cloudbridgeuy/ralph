use super::redact::{mask_key, redacts, REDACTED};
use super::{
    env_settings, leaf_sources, read_layer, winning_override, AppConfig, ConfigError, Row, Setting,
    Source,
};
use serde::Serialize;
use std::path::Path;
use toml::Table;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RowSource {
    Default,
    Global,
    Store,
    Env,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigRow {
    pub section: String,
    pub key: String,
    pub value: serde_json::Value,
    pub source: RowSource,
    pub redacted: bool,
}

fn split_key(key: &str) -> (String, String) {
    match key.split_once('.') {
        Some((section, rest)) => (section.to_string(), rest.to_string()),
        None => ("root".to_string(), key.to_string()),
    }
}

fn source_of(source: Source) -> RowSource {
    match source {
        Source::Default => RowSource::Default,
        Source::Global => RowSource::Global,
        Source::Store => RowSource::Store,
    }
}

fn row(full_key: &str, value: serde_json::Value, source: RowSource) -> ConfigRow {
    let (section, key) = split_key(&mask_key(full_key));
    let redacted = redacts(full_key, &value, source == RowSource::Env);
    ConfigRow {
        section,
        key,
        value: if redacted {
            serde_json::Value::String(REDACTED.to_string())
        } else {
            value
        },
        source,
        redacted,
    }
}

fn json(value: &toml::Value) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

fn build_rows(layered: &[Row], overrides: &[Setting]) -> Vec<ConfigRow> {
    let from_layers = layered
        .iter()
        .map(|layer| match winning_override(overrides, &layer.key) {
            Some(setting) => row(&layer.key, json(&setting.value), RowSource::Env),
            None => row(&layer.key, json(&layer.value), source_of(layer.source)),
        });
    let env_only = overrides.iter().filter_map(|setting| {
        let key = setting.key?;
        (!layered.iter().any(|layer| layer.key == key))
            .then(|| row(key, json(&setting.value), RowSource::Env))
    });
    from_layers.chain(env_only).collect()
}

pub fn config_rows(
    global: &Path,
    store: &Path,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Vec<ConfigRow>, ConfigError> {
    let store_file = store.join("config.toml");
    let config = AppConfig::load_layers(global, Some(&store_file))?;
    let effective = Table::try_from(&config)?;
    let layered = leaf_sources(&effective, &read_layer(global)?, &read_layer(&store_file)?);
    Ok(build_rows(&layered, &env_settings(&[], env)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const FIXTURE: &str = include_str!("../../fixtures/config.toml");

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn find<'a>(rows: &'a [ConfigRow], section: &str, key: &str) -> &'a ConfigRow {
        rows.iter()
            .find(|row| row.section == section && row.key == key)
            .unwrap()
    }

    fn dirs_with(global: Option<&str>, store: Option<&str>) -> (TempDir, TempDir) {
        let g = TempDir::new().unwrap();
        let s = TempDir::new().unwrap();
        if let Some(text) = global {
            std::fs::write(g.path().join("config.toml"), text).unwrap();
        }
        if let Some(text) = store {
            std::fs::write(s.path().join("config.toml"), text).unwrap();
        }
        (g, s)
    }

    fn rows(g: &TempDir, s: &TempDir, env: impl Fn(&str) -> Option<String>) -> Vec<ConfigRow> {
        config_rows(&g.path().join("config.toml"), s.path(), env).unwrap()
    }

    #[test]
    fn secrets_never_reach_the_serialized_rows() {
        let (g, s) = dirs_with(Some(FIXTURE), None);
        let rows = rows(&g, &s, |name| {
            (name == "RALPH_THEME").then(|| "sk-secret".to_string())
        });
        let text = serde_json::to_string(&rows).unwrap();
        for secret in ["abc123", "/Users", "sk-secret"] {
            assert!(!text.contains(secret), "{secret}");
        }
        assert!(find(&rows, "claude", "shell").redacted);
        assert!(find(&rows, "usage", "ledger").redacted);
        assert!(!find(&rows, "chat", "wrap_width").redacted);
    }

    #[test]
    fn sensitive_key_names_never_reach_the_serialized_rows() {
        let (g, s) = dirs_with(Some("[wire.prices.\"/Users/me/srv\"]\ninput = 1\n"), None);
        let text = serde_json::to_string(&rows(&g, &s, no_env)).unwrap();
        assert!(!text.contains("/Users"), "{text}");
        assert!(text.contains("[redacted]"));
    }

    #[test]
    fn missing_files_give_defaults_only() {
        let (g, s) = dirs_with(None, None);
        let rows = rows(&g, &s, no_env);
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.source == RowSource::Default));
    }

    #[test]
    fn global_values_carry_the_global_source() {
        let (g, s) = dirs_with(Some(FIXTURE), None);
        let rows = rows(&g, &s, no_env);
        let wrap = find(&rows, "chat", "wrap_width");
        assert_eq!(wrap.source, RowSource::Global);
        assert_eq!(wrap.value, serde_json::json!(96));
        assert_eq!(find(&rows, "web", "port").source, RowSource::Global);
    }

    #[test]
    fn store_values_win_and_carry_the_store_source() {
        let (g, s) = dirs_with(Some(FIXTURE), Some("[chat]\nwrap_width = 120\n"));
        let rows = rows(&g, &s, no_env);
        let wrap = find(&rows, "chat", "wrap_width");
        assert_eq!(wrap.source, RowSource::Store);
        assert_eq!(wrap.value, serde_json::json!(120));
        assert_eq!(
            find(&rows, "chat", "cell_output_limit").source,
            RowSource::Global
        );
    }

    #[test]
    fn env_values_win_and_carry_the_env_source() {
        let (g, s) = dirs_with(Some(FIXTURE), None);
        let rows = rows(&g, &s, |name| {
            (name == "RALPH_WRAP_WIDTH").then(|| "72".to_string())
        });
        let wrap = find(&rows, "chat", "wrap_width");
        assert_eq!(wrap.source, RowSource::Env);
        assert!(wrap.redacted);
    }

    #[test]
    fn keys_split_into_section_and_rest() {
        let (g, s) = dirs_with(Some(FIXTURE), None);
        let rows = rows(&g, &s, no_env);
        let shell = find(&rows, "claude", "shell");
        assert_eq!(shell.source, RowSource::Global);
        assert!(rows
            .iter()
            .all(|row| !row.section.is_empty() && !row.key.is_empty()));
    }

    #[test]
    fn keys_without_a_section_use_root() {
        assert_eq!(
            super::split_key("verbose"),
            ("root".to_string(), "verbose".to_string())
        );
        assert_eq!(
            super::split_key("theme.assistant.code"),
            ("theme".to_string(), "assistant.code".to_string())
        );
    }

    #[test]
    fn a_sensitive_segment_redacts_the_whole_key_tail_and_value() {
        let (g, s) = dirs_with(
            None,
            Some("[wire.prices.\"/Users/alice/acme.corp\"]\ninput = 1\n"),
        );
        let rows = rows(&g, &s, no_env);
        let body = serde_json::to_string(&rows).unwrap();
        assert!(!body.contains("corp"), "{body}");
        assert!(!body.contains("/Users"), "{body}");
        let masked = find(&rows, "wire", "[redacted]");
        assert!(masked.redacted);
        assert_eq!(masked.value, serde_json::json!("[redacted]"));
    }

    #[test]
    fn an_array_of_tables_with_a_path_key_is_redacted_in_rows() {
        let effective: Table = "[web]\nfont = [{\"/Users/x\" = 1}]\n".parse().unwrap();
        let empty = Table::new();
        let layered = leaf_sources(&effective, &empty, &empty);
        let rows = build_rows(&layered, &[]);
        let font = find(&rows, "web", "font");
        assert!(font.redacted);
        let body = serde_json::to_string(&rows).unwrap();
        assert!(!body.contains("/Users"), "{body}");
    }

    #[test]
    fn invalid_toml_is_a_parse_error() {
        let (g, s) = dirs_with(Some("[chat\nwrap_width = "), None);
        let result = config_rows(&g.path().join("config.toml"), s.path(), no_env);
        assert!(matches!(result, Err(ConfigError::ParseConfig { .. })));
    }
}
