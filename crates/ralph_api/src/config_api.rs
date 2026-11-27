use crate::route::{error, json_ok, json_response, ApiConfig, ApiRequest, ApiResponse};
use ralph_server::config::{config_rows, ConfigError};
use serde_json::json;
use std::path::Path;

pub(crate) fn store_name(store: &Path) -> String {
    store.file_name().map_or_else(
        || "store".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

const CONFIG_INVALID_WHY: &str = "config.toml has an error; run `ralph config check`";

fn failure(error_value: &ConfigError) -> ApiResponse {
    match error_value {
        ConfigError::ReadConfig { .. } | ConfigError::SerializeConfig(_) => {
            error(500, "config unavailable")
        }
        _ => json_response(
            422,
            json!({"error": "config invalid", "why": CONFIG_INVALID_WHY}),
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
        assert_eq!(value["why"], CONFIG_INVALID_WHY);
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
    fn a_quoted_price_name_with_a_space_never_reaches_the_why() {
        let (g, s) = setup(
            None,
            Some("[wire.prices.\"/Users/me/My Srv\"]\ninput = -1\n"),
        );
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 422);
        let all = text(&response);
        assert!(!all.contains("Srv"), "{all}");
        assert!(!all.contains("/Users"), "{all}");
        assert_eq!(body(&response)["why"], CONFIG_INVALID_WHY);
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
        assert_eq!(body(&response)["why"], CONFIG_INVALID_WHY);
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
        assert_eq!(body(&response)["why"], CONFIG_INVALID_WHY);
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
    fn dotted_sensitive_price_key_never_reaches_the_body() {
        let (g, s) = setup(
            None,
            Some("[wire.prices.\"/Users/alice/acme.corp\"]\ninput = 1\n"),
        );
        let response = respond(&g, &s, no_env);
        let raw = text(&response);
        assert!(!raw.contains("corp"), "{raw}");
        assert!(!raw.contains("/Users"), "{raw}");
        let masked = row(&body(&response), "wire", "[redacted-1]").clone();
        assert_eq!(masked["redacted"], true, "{raw}");
        assert_eq!(masked["value"], "[redacted]", "{raw}");
    }

    #[test]
    fn windows_and_variable_path_keys_never_reach_the_body() {
        for (key, leaks) in [
            (r"C:\\Users\\alice\\acme", ["alice", "acme", "Users"]),
            (r"\\\\host\\share", ["host", "share", "host"]),
            ("D:/x", ["D:", "/x", "D:"]),
            ("%USERPROFILE%", ["USERPROFILE", "%", "USERPROFILE"]),
        ] {
            let (g, s) = setup(None, Some(&format!("[wire.prices.\"{key}\"]\ninput = 1\n")));
            let response = respond(&g, &s, no_env);
            let raw = text(&response);
            for leak in leaks {
                assert!(!raw.contains(leak), "{key}: {raw}");
            }
            let masked = row(&body(&response), "wire", "[redacted-1]").clone();
            assert_eq!(masked["redacted"], true, "{raw}");
            assert_eq!(masked["value"], "[redacted]", "{raw}");
        }
    }

    #[test]
    fn a_colon_price_key_stays_visible() {
        let (g, s) = setup(None, Some("[wire.prices.\"x:1\"]\ninput = 1\n"));
        let value = body(&respond(&g, &s, no_env));
        assert_eq!(row(&value, "wire", "prices.x:1.input")["redacted"], false);
    }

    #[test]
    fn bare_secrets_in_tight_fields_never_reach_the_body() {
        for (config, leak) in [
            ("[mermaid.style.colors]\nx = \"ghp_SECRET\"\n", "ghp_SECRET"),
            (
                "[mermaid.style.config]\nnote = \"correct horse staple\"\n",
                "horse",
            ),
            ("[tickets]\nlinear = [\"my secret ticket\"]\n", "secret"),
            ("[wire.prices.\"hunter2 secret\"]\ninput = 1\n", "hunter2"),
        ] {
            let (g, s) = setup(None, Some(config));
            let response = respond(&g, &s, no_env);
            let raw = text(&response);
            assert!(!raw.contains(leak), "{config}: {raw}");
            let hidden = body(&response)["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["redacted"] == true && row["value"] == "[redacted]");
            assert!(hidden, "{config}: {raw}");
        }
    }

    #[test]
    fn opaque_secrets_in_loose_fields_never_reach_the_body() {
        for (config, leak) in [
            ("[claude]\nmodel = \"ghp_SECRET\"\n", "ghp_SECRET"),
            ("[chat]\nname = \"Bearer abc123\"\n", "abc123"),
            ("[chat]\nname = \"xo\x78b-123-abc\"\n", "xoxb"),
            (
                "[web]\nfont = \"aGVsbG8gd29ybGQgc2VjcmV0MTI1\"\n",
                "aGVsbG8",
            ),
        ] {
            let (g, s) = setup(None, Some(config));
            let response = respond(&g, &s, no_env);
            let raw = text(&response);
            assert!(!raw.contains(leak), "{config}: {raw}");
            let hidden = body(&response)["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["redacted"] == true && row["value"] == "[redacted]");
            assert!(hidden, "{config}: {raw}");
        }
    }

    fn assert_key_absent_and_masked(config: &str, leak: &str, section: &str) {
        let (g, s) = setup(None, Some(config));
        let response = respond(&g, &s, no_env);
        assert_eq!(response.status, 200, "{config}");
        let raw = text(&response);
        assert!(!raw.contains(leak), "{config}: {raw}");
        let masked = row(&body(&response), section, "[redacted-1]").clone();
        assert_eq!(masked["redacted"], true, "{raw}");
        assert_eq!(masked["value"], "[redacted]", "{raw}");
    }

    #[test]
    fn free_text_map_keys_never_reach_the_body() {
        for (config, leak, section) in [
            (
                "[mermaid.style.colors]\n\"hunter2 secret\" = \"#fff\"\n",
                "hunter2",
                "mermaid",
            ),
            (
                "[theme.status]\n\"my password is hunter2\" = \"red\"\n",
                "password",
                "theme",
            ),
            (
                "[mermaid.style.config.flowchart]\n\"correct horse battery staple\" = 30\n",
                "horse",
                "mermaid",
            ),
            (
                "[claude.permissions]\n\"secret words\" = true\n",
                "secret words",
                "claude",
            ),
            (
                "[chat.reset_turns]\n\"secret words\" = 1\n",
                "secret words",
                "chat",
            ),
        ] {
            assert_key_absent_and_masked(config, leak, section);
        }
    }

    #[test]
    fn split_and_punctuated_opaque_tokens_never_reach_the_body() {
        for (config, leak) in [
            ("[chat]\nname = \"pass:AbCdEf0123456789xyzQ\"\n", "AbCdEf"),
            ("[chat]\nname = \"AbCdEf0123456789xyzQ.\"\n", "AbCdEf"),
            (
                "[chat]\nname = \"Zx9k2Lq-8vB3mN0pQrS-7tUwXyZ1aB2cD3e\"\n",
                "Zx9k2Lq",
            ),
            (
                "[chat]\nname = \"550e8400-e29b-41d4-a716-446655440000\"\n",
                "550e8400",
            ),
            (
                "[wire.prices.\"AbCdEf0123456.789xyzQAbCd\"]\ninput = 1\n",
                "AbCdEf",
            ),
            ("[theme]\nname = \"AbCdEf0123.456789xyzQAbCd\"\n", "AbCdEf"),
        ] {
            let (g, s) = setup(None, Some(config));
            let raw = text(&respond(&g, &s, no_env));
            assert!(!raw.contains(leak), "{config}: {raw}");
        }
    }

    #[test]
    fn a_dated_model_name_stays_visible_in_the_body() {
        let (g, s) = setup(
            None,
            Some("[claude]\nmodel = \"claude-opus-4-1-20250805\"\n"),
        );
        let value = body(&respond(&g, &s, no_env));
        assert_eq!(
            row(&value, "claude", "model")["value"],
            "claude-opus-4-1-20250805"
        );
    }

    #[test]
    fn text_in_a_numeric_setting_never_reaches_the_body() {
        for (config, leak) in [
            (
                "[chat]\nreset_turns = \"my password is hunter2\"\n",
                "hunter2",
            ),
            ("[chat]\nreset_tokens = [\"my secret words\"]\n", "secret"),
            (
                "[chat]\nworker_quiet_secs = \"my password is hunter2\"\n",
                "hunter2",
            ),
            (
                "[chat.reset_turns]\nx = \"my password is hunter2\"\n",
                "hunter2",
            ),
        ] {
            let (g, s) = setup(None, Some(config));
            let response = respond(&g, &s, no_env);
            assert_eq!(response.status, 200, "{config}");
            let raw = text(&response);
            assert!(!raw.contains(leak), "{config}: {raw}");
            let hidden = body(&response)["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["redacted"] == true && row["value"] == "[redacted]");
            assert!(hidden, "{config}: {raw}");
        }
    }

    #[test]
    fn a_number_in_a_numeric_setting_stays_visible_in_the_body() {
        let (g, s) = setup(None, Some("[chat]\nreset_turns = 40\n"));
        let value = body(&respond(&g, &s, no_env));
        let found = row(&value, "chat", "reset_turns");
        assert_eq!(found["value"], 40);
        assert_eq!(found["redacted"], false);
    }

    const PUNCTUATED_SECRETS: [&str; 6] = [
        "AbCdEf0123456789xyzQ!",
        "#AbCdEf0123456789xyzQ",
        "x*AbCdEf0123456789xyzQ",
        "AbCdEf0123|456789xyzQAbCd",
        "AbCdEf0123\u{200B}456789xyzQAbCd",
        "9f86d081-884c7d65-9a2feaa0-c55ad015-a3bf4f1b",
    ];

    #[test]
    fn punctuated_and_dashed_secrets_in_loose_and_ident_fields_never_reach_the_body() {
        for secret in PUNCTUATED_SECRETS {
            for config in [
                format!("[chat]\nname = \"{secret}\"\n"),
                format!("[tickets]\nlinear = [\"{secret}\"]\n"),
            ] {
                let (g, s) = setup(None, Some(&config));
                let response = respond(&g, &s, no_env);
                let raw = text(&response);
                for leak in ["AbCdEf", "9f86d081", "884c7d65"] {
                    assert!(!raw.contains(leak), "{config}: {raw}");
                }
                let hidden = body(&response)["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["redacted"] == true && row["value"] == "[redacted]");
                assert!(hidden, "{config}: {raw}");
            }
        }
    }

    #[test]
    fn the_default_font_and_a_dated_model_stay_visible_in_the_body() {
        let (g, s) = setup(None, Some("[claude]\nmodel = \"gpt-4o-2024-08-06\"\n"));
        let value = body(&respond(&g, &s, no_env));
        assert_eq!(row(&value, "claude", "model")["value"], "gpt-4o-2024-08-06");
        let value = body(&respond(&g, &s, no_env));
        let font = row(&value, "web", "font");
        assert_eq!(font["redacted"], false);
        assert_eq!(
            font["value"],
            "\"Geist Mono\", ui-monospace, Menlo, monospace"
        );
    }
}
