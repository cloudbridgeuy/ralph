use crate::route::{error, json_ok, json_response, ApiConfig, ApiRequest, ApiResponse};
use ralph_server::config::{config_rows, ConfigError};
use serde_json::json;
use std::path::Path;

const PATH_MARK: &str = "<path>";

fn store_name(store: &Path) -> String {
    store.file_name().map_or_else(
        || "store".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn is_path_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>')
}

fn mask_path(word: &str) -> String {
    let start = word.find(|c: char| is_path_char(c)).unwrap_or(word.len());
    let (lead, rest) = word.split_at(start);
    if !rest.starts_with('/') {
        return word.to_string();
    }
    let end = rest.find(|c: char| !is_path_char(c)).unwrap_or(rest.len());
    let (path, tail) = rest.split_at(end);
    let trimmed = path.trim_end_matches([',', ';', ':', '.']);
    let punct = &path[trimmed.len()..];
    format!("{lead}{PATH_MARK}{punct}{}", mask_path(tail))
}

fn redact_quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (inside, c) {
            (false, '"') => {
                inside = true;
                out.push('"');
                out.push_str("[redacted]");
            }
            (false, other) => out.push(other),
            (true, '\\') => {
                chars.next();
            }
            (true, '"') => {
                inside = false;
                out.push('"');
            }
            (true, _) => {}
        }
    }
    out
}

pub(crate) fn why_text(display: &str) -> String {
    let first = display.lines().next().unwrap_or_default();
    mask_words(&redact_quoted(first))
}

fn spaced_path_end(words: &[&str]) -> Option<usize> {
    let first = words.first()?;
    let start = first.find(is_path_char)?;
    let path = &first[start..];
    if !path.starts_with('/') || path.ends_with([',', ';', ':', '.', ')']) {
        return None;
    }
    words.iter().position(|word| word.ends_with(':'))
}

fn mask_words(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    let mut out = Vec::with_capacity(words.len());
    let mut index = 0;
    while index < words.len() {
        match spaced_path_end(&words[index..]) {
            Some(offset) => {
                let word = words[index];
                let lead = &word[..word.find(is_path_char).unwrap_or(0)];
                out.push(format!("{lead}{PATH_MARK}:"));
                index += offset + 1;
            }
            None => {
                out.push(mask_path(words[index]));
                index += 1;
            }
        }
    }
    out.join(" ")
}

fn failure(error_value: &ConfigError) -> ApiResponse {
    match error_value {
        ConfigError::ReadConfig { .. } | ConfigError::SerializeConfig(_) => {
            error(500, "config unavailable")
        }
        other => json_response(
            422,
            json!({"error": "config invalid", "why": why_text(&other.to_string())}),
        ),
    }
}

pub(crate) fn config_response(
    global: &Path,
    store: &Path,
    env: impl Fn(&str) -> Option<String>,
) -> ApiResponse {
    if !store.is_dir() {
        return error(404, "store not found");
    }
    match config_rows(global, store, env) {
        Ok(rows) => json_ok(json!({"store": store_name(store), "rows": rows})),
        Err(found) => failure(&found),
    }
}

pub(crate) fn config(_request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    config_response(&config.global_config, &config.store, config.env)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::fs;
    use tempfile::TempDir;

    const FIXTURE: &str = include_str!("../../ralph_server/fixtures/config.toml");

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn body(response: &ApiResponse) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    fn text(response: &ApiResponse) -> String {
        String::from_utf8(response.body.clone()).unwrap()
    }

    fn setup(global: Option<&str>, store: Option<&str>) -> (TempDir, TempDir) {
        let g = TempDir::new().unwrap();
        let s = TempDir::new().unwrap();
        if let Some(content) = global {
            fs::write(g.path().join("config.toml"), content).unwrap();
        }
        if let Some(content) = store {
            fs::write(s.path().join("config.toml"), content).unwrap();
        }
        (g, s)
    }

    fn respond(g: &TempDir, s: &TempDir, env: impl Fn(&str) -> Option<String>) -> ApiResponse {
        config_response(&g.path().join("config.toml"), s.path(), env)
    }

    fn row<'a>(value: &'a Value, section: &str, key: &str) -> &'a Value {
        value["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["section"] == section && row["key"] == key)
            .unwrap()
    }

    #[test]
    fn ok_has_store_name_and_row_shape() {
        let (g, s) = setup(None, Some(FIXTURE));
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 200);
        let value = body(&response);
        let name = s.path().file_name().unwrap().to_str().unwrap();
        assert_eq!(value["store"], name);
        let theme = row(&value, "theme", "name");
        assert_eq!(theme["value"], "tokyo-night");
        assert_eq!(theme["source"], "store");
        assert_eq!(theme["redacted"], false);
    }

    #[test]
    fn sources_cover_default_global_store_env() {
        let (g, s) = setup(Some("[chat]\nwrap_width = 70\n"), Some(FIXTURE));
        let response = respond(&g, &s, |name| {
            (name == "RALPH_THEME").then(|| "dracula".to_string())
        });
        let value = body(&response);
        assert_eq!(row(&value, "chat", "wrap_width")["source"], "store");
        assert_eq!(row(&value, "theme", "name")["source"], "env");
        assert_eq!(row(&value, "theme", "name")["redacted"], true);
        let sources: Vec<&str> = value["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["source"].as_str().unwrap())
            .collect();
        assert!(sources.contains(&"default"));
    }

    #[test]
    fn global_source_is_reported() {
        let (g, s) = setup(Some("[chat]\nwrap_width = 70\n"), None);
        let value = body(&respond(&g, &s, no_env));
        assert_eq!(row(&value, "chat", "wrap_width")["source"], "global");
    }

    #[test]
    fn secrets_and_paths_never_reach_the_body() {
        let (g, s) = setup(None, Some(FIXTURE));
        let response = respond(&g, &s, no_env);
        let all = text(&response);
        for leak in [
            "abc123",
            "/Users",
            s.path().to_str().unwrap(),
            g.path().to_str().unwrap(),
        ] {
            assert!(!all.contains(leak), "{leak}");
        }
        let value = body(&response);
        assert_eq!(row(&value, "claude", "shell")["redacted"], true);
    }

    #[test]
    fn missing_config_file_is_defaults() {
        let (g, s) = setup(None, None);
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 200);
        assert!(!body(&response)["rows"].as_array().unwrap().is_empty());
    }

    #[test]
    fn missing_store_is_404() {
        let (g, s) = setup(None, None);
        let gone = s.path().join("gone");
        let response = config_response(&g.path().join("config.toml"), &gone, no_env);
        assert_eq!(response.status, 404);
        assert_eq!(body(&response), json!({"error": "store not found"}));
    }

    #[test]
    fn invalid_toml_is_422_without_paths_or_content() {
        let (g, s) = setup(None, Some("[claude]\nshell = \"X_TOKEN=abc123\n"));
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 422);
        let value = body(&response);
        assert_eq!(value["error"], "config invalid");
        let all = text(&response);
        assert!(!all.contains(s.path().to_str().unwrap()));
        assert!(!all.contains("abc123"));
        assert!(!value["why"].as_str().unwrap().is_empty());
    }

    #[test]
    fn invalid_value_is_422() {
        let (g, s) = setup(None, Some("[web]\nport = 99999\n"));
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 422);
        assert!(!text(&response).contains(s.path().to_str().unwrap()));
    }

    #[test]
    fn unreadable_config_is_500() {
        let (g, s) = setup(None, None);
        fs::create_dir(s.path().join("config.toml")).unwrap();
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 500);
        assert_eq!(body(&response), json!({"error": "config unavailable"}));
    }

    #[test]
    fn why_strips_absolute_paths() {
        let why = why_text("Failed to read at /Users/x/.ralph/config.toml: bad value");
        assert_eq!(why, "Failed to read at <path>: bad value");
    }

    #[test]
    fn why_strips_every_path_and_quoted_paths() {
        let why = why_text("a /one/two, b \"/three/four\" c (/five)");
        assert_eq!(why, "a <path>, b \"[redacted]\" c (<path>)");
    }

    #[test]
    fn typed_value_errors_never_quote_the_value() {
        let (g, s) = setup(None, Some("[web]\nport = \"sk-abc123\"\n"));
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 422);
        assert!(
            !text(&response).contains("sk-abc123"),
            "{}",
            text(&response)
        );
    }

    #[test]
    fn parse_errors_never_quote_a_secret_line() {
        let (g, s) = setup(None, Some("[claude]\nshell = \"sk-abc123\" oops\n"));
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 422);
        assert!(
            !text(&response).contains("sk-abc123"),
            "{}",
            text(&response)
        );
    }

    #[test]
    fn why_redacts_quoted_spans() {
        assert_eq!(
            why_text("invalid type: string \"sk-abc\", expected u16"),
            "invalid type: string \"[redacted]\", expected u16"
        );
        assert_eq!(
            why_text("bad \"a\\\"b\" then \"c\""),
            "bad \"[redacted]\" then \"[redacted]\""
        );
    }

    #[test]
    fn why_masks_a_path_with_spaces_up_to_the_colon() {
        let why = why_text(
            "Failed to read /Users/x/Library/Application Support/ralph/config.toml: bad value",
        );
        assert_eq!(why, "Failed to read <path>: bad value");
        assert!(!why.contains("Support"));
    }

    fn text_contains_row(value: &Value, key: &str) -> bool {
        value["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["key"] == key)
    }

    fn theme_env(name: &str) -> Option<String> {
        (name == "RALPH_THEME").then(|| "dracula".to_string())
    }

    fn handler_config(
        global: &TempDir,
        store: &TempDir,
        env: fn(&str) -> Option<String>,
    ) -> ApiConfig {
        ApiConfig {
            store: store.path().to_path_buf(),
            allowed_hosts: Vec::new(),
            version: "0".to_string(),
            tokens_css: String::new(),
            cookie_name: "c".to_string(),
            global_config: global.path().join("config.toml"),
            env,
        }
    }

    fn get_request() -> ApiRequest {
        ApiRequest {
            method: "GET".to_string(),
            path: "/api/config".to_string(),
            query: String::new(),
            host: None,
            authorization: None,
            cookies: Vec::new(),
            body: Vec::new(),
            content_type: None,
            body_too_large: false,
        }
    }

    #[test]
    fn handler_uses_the_injected_env_and_global_path() {
        let (g, s) = setup(Some("[chat]\nwrap_width = 70\n"), None);
        let value = body(&config(&get_request(), &handler_config(&g, &s, theme_env)));
        assert_eq!(row(&value, "theme", "name")["source"], "env");
        assert_eq!(row(&value, "theme", "name")["redacted"], true);
        assert_eq!(row(&value, "chat", "wrap_width")["source"], "global");
        let (g2, s2) = setup(None, None);
        let value = body(&config(&get_request(), &handler_config(&g2, &s2, no_env)));
        assert!(!text_contains_row(&value, "wrap_width"));
    }

    #[test]
    fn why_keeps_first_line_only() {
        let why = why_text("parse error at line 1\n  |\n1 | token = abc123");
        assert_eq!(why, "parse error at line 1");
    }

    #[test]
    fn why_keeps_text_without_paths() {
        assert_eq!(why_text("port must be 1-65535"), "port must be 1-65535");
        assert_eq!(why_text("a and/or b"), "a and/or b");
    }
}
