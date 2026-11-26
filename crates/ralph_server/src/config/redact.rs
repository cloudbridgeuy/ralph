use super::redact_shapes::{
    key_class, key_shape_ok, text_has_path_shape, text_has_secret_shape, value_shape_ok,
};
use serde_json::Value;

pub const REDACTED: &str = "[redacted]";

const REDACT_KEYS: &[&str] = &[
    "chat.skills",
    "chat.prompt_file",
    "chat.append_prompt_files",
    "chat.worktree_dir",
    "claude.config_dir",
    "usage.ledger",
    "claude.shell",
    "workers.shell",
    "chat.editor",
    "chat.send_it_verify",
    "chat.mcp_servers",
    "chat.empty_prompt",
];

const SAFE_KEYS: &[&str] = &[
    "changes.diff_args",
    "changes.enabled",
    "changes.pager",
    "chat.auto_continue",
    "chat.auto_hold",
    "chat.cell_output_bytes",
    "chat.cell_output_limit",
    "chat.edit_mode",
    "chat.emphasis",
    "chat.emphasis_icons",
    "chat.emphasis_round",
    "chat.empty_submit",
    "chat.hold_progress",
    "chat.inbox_alert_notification",
    "chat.inbox_alert_sound",
    "chat.inbox_alert_unknown_focus",
    "chat.keepalive_hours",
    "chat.project_claude_md",
    "chat.recent_history_tokens",
    "chat.reset_tokens",
    "chat.reset_turns",
    "chat.resume_summary",
    "chat.send_it_max_turns",
    "chat.send_it_max_usd",
    "chat.status_powerline",
    "chat.tag_entries",
    "chat.terminal_status",
    "chat.terminal_title",
    "chat.user_claude_md",
    "chat.user_settings",
    "chat.watch_timeout_secs",
    "chat.worker_base",
    "chat.worker_check_secs",
    "chat.worker_quiet_secs",
    "chat.wrap_width",
    "nap.harness",
    "nap.model",
    "tag.harness",
    "tag.model",
    "workers.pr_provider",
    "mermaid.style",
    "wire.prices",
    "chat.name",
    "opencode.model",
    "mermaid.max_class_relations",
    "mermaid.max_flowchart_edges",
    "mermaid.max_input_bytes",
    "mermaid.max_rss_mb",
    "mermaid.picture_timeout_ms",
    "mermaid.timeout_ms",
    "web.ansi_dark",
    "web.ansi_light",
    "web.font",
    "web.port",
    "workers.cache_secs",
    "workers.lookup_timeout_ms",
    "workers.verify_on_drift",
    "workers.view",
    "agy.effort",
    "codex.effort",
    "agy.model",
    "codex.model",
    "cursor.model",
    "claude.model",
    "claude.effort",
];

const SAFE_PREFIXES: &[&str] = &["theme.", "mermaid.style.", "wire.prices.", "tickets."];

const VALUE_MARKERS: [&str; 4] = ["://", "@", "key=", "token="];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    Redact,
    Safe,
    Unlisted,
}

fn is_permission_key(key: &str) -> bool {
    key.split('.')
        .any(|segment| segment == "sandbox" || segment.starts_with("permission"))
}

pub fn listing(key: &str) -> Listing {
    if REDACT_KEYS.contains(&key) || is_permission_key(key) {
        Listing::Redact
    } else if SAFE_KEYS.contains(&key) || SAFE_PREFIXES.iter().any(|p| key.starts_with(p)) {
        Listing::Safe
    } else {
        Listing::Unlisted
    }
}

fn text_is_sensitive(text: &str) -> bool {
    text_has_path_shape(text)
        || text_has_secret_shape(text)
        || VALUE_MARKERS.iter().any(|marker| text.contains(marker))
}

fn key_is_sensitive(key: &str) -> bool {
    key.split('.').any(text_is_sensitive) || !key_shape_ok(key)
}

pub fn mask_key(key: &str) -> String {
    if !key_is_sensitive(key) {
        return key.to_string();
    }
    match key.split_once('.') {
        Some((section, _)) if !text_is_sensitive(section) => format!("{section}.{REDACTED}"),
        _ => REDACTED.to_string(),
    }
}

fn value_is_sensitive(value: &Value) -> bool {
    match value {
        Value::String(text) => text_is_sensitive(text),
        Value::Array(items) => items.iter().any(value_is_sensitive),
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| text_is_sensitive(key) || value_is_sensitive(value)),
        _ => false,
    }
}

pub fn redacts(key: &str, value: &Value, from_env: bool) -> bool {
    from_env
        || listing(key) != Listing::Safe
        || key_is_sensitive(key)
        || value_is_sensitive(value)
        || !value_shape_ok(key_class(key), value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use serde_json::json;
    use toml::{Table, Value as Toml};

    fn leaf_keys(table: &Table, prefix: &str, keys: &mut Vec<String>) {
        for (name, value) in table {
            let key = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            match value {
                Toml::Table(inner) => leaf_keys(inner, &key, keys),
                _ => keys.push(key),
            }
        }
    }

    #[test]
    fn every_default_leaf_key_is_listed() {
        let table = Table::try_from(AppConfig::new()).unwrap();
        let mut keys = Vec::new();
        leaf_keys(&table, "", &mut keys);
        assert!(!keys.is_empty());
        let unlisted: Vec<&String> = keys
            .iter()
            .filter(|key| listing(key) == Listing::Unlisted)
            .collect();
        assert!(unlisted.is_empty(), "unlisted keys: {unlisted:?}");
    }

    const SCHEMA_SOURCES: [&str; 11] = [
        include_str!("../config.rs"),
        include_str!("agy.rs"),
        include_str!("changes.rs"),
        include_str!("claude.rs"),
        include_str!("codex.rs"),
        include_str!("cursor.rs"),
        include_str!("opencode.rs"),
        include_str!("usage.rs"),
        include_str!("web.rs"),
        include_str!("wire.rs"),
        include_str!("workers.rs"),
    ];

    fn section_field_keys(source: &str) -> Vec<String> {
        let mut section: Option<String> = None;
        let mut keys = Vec::new();
        for line in source.lines() {
            if let Some(rest) = line.strip_prefix("pub struct ") {
                section = rest
                    .split(|c: char| !c.is_alphanumeric())
                    .next()
                    .and_then(|name| name.strip_suffix("Section"))
                    .map(str::to_lowercase);
                continue;
            }
            let Some(section) = &section else { continue };
            let field = line
                .trim()
                .strip_prefix("pub ")
                .and_then(|r| r.split_once(": "))
                .filter(|(name, _)| name.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            if let Some((name, ty)) = field {
                let ty = ty.trim_end_matches(',');
                if !ty.ends_with("Section") && !ty.ends_with("Theme") {
                    keys.push(format!("{section}.{name}"));
                }
            }
        }
        keys
    }

    #[test]
    fn every_declared_section_field_has_a_listing() {
        let keys: Vec<String> = SCHEMA_SOURCES
            .iter()
            .flat_map(|source| section_field_keys(source))
            .collect();
        assert!(keys.len() > 50, "scan found {} fields", keys.len());
        let missing: Vec<&String> = keys
            .iter()
            .filter(|key| listing(key) == Listing::Unlisted)
            .collect();
        assert!(missing.is_empty(), "fields with no listing: {missing:?}");
    }

    #[test]
    fn listed_secret_keys_are_redacted_whatever_the_value() {
        for key in REDACT_KEYS {
            assert!(redacts(key, &json!("plain"), false), "{key}");
        }
    }

    #[test]
    fn permission_and_sandbox_keys_are_redacted() {
        for key in [
            "claude.permissions",
            "codex.sandbox",
            "agy.permissions",
            "cursor.permissions",
            "chat.permissions",
            "chat.claude.permission_mode",
            "ask.claude.permission_mode",
            "ask.permissions",
        ] {
            assert_eq!(listing(key), Listing::Redact, "{key}");
        }
    }

    #[test]
    fn env_values_are_always_redacted() {
        assert!(redacts("chat.wrap_width", &json!("72"), true));
    }

    #[test]
    fn safe_keys_keep_plain_values() {
        assert!(!redacts("chat.wrap_width", &json!(96), false));
        assert!(!redacts("theme.name", &json!("tokyo-night"), false));
        assert!(!redacts(
            "web.font",
            &json!("\"Geist Mono\", monospace"),
            false
        ));
    }

    #[test]
    fn sensitive_looking_values_are_redacted_on_safe_keys() {
        for text in [
            "/Users/x",
            "~/notes",
            "https://example.com",
            "a@b",
            "key=1",
            "token=1",
        ] {
            assert!(redacts("theme.name", &json!(text), false), "{text}");
        }
        assert!(redacts("web.ansi_dark", &json!(["#fff", "/etc"]), false));
    }

    #[test]
    fn safe_keys_redact_paths_anywhere_in_the_value() {
        for (key, value) in [
            (
                "changes.diff_args",
                json!(["--git-dir=/Users/me/work/.git"]),
            ),
            ("chat.name", json!("work in ~/me")),
            ("web.font", json!("a/b")),
            ("chat.worker_base", json!("--base=/Users/me")),
            ("mermaid.style.node", json!(["x", "y=/etc/z"])),
        ] {
            assert!(redacts(key, &value, false), "{key}");
        }
    }

    #[test]
    fn mask_key_collapses_the_tail_when_any_segment_is_sensitive() {
        assert_eq!(mask_key("chat.wrap_width"), "chat.wrap_width");
        assert_eq!(
            mask_key("wire.prices./Users/me/srv.input"),
            "wire.[redacted]"
        );
        assert_eq!(
            mask_key("wire.prices./Users/alice/acme.corp.input"),
            "wire.[redacted]"
        );
        assert_eq!(mask_key("a.b@c.d"), "a.[redacted]");
        assert_eq!(mask_key("/etc.x"), "[redacted]");
    }

    #[test]
    fn sensitive_key_segments_redact_the_value() {
        assert!(redacts("wire.prices./Users/me/srv.input", &json!(1), false));
        assert!(!redacts("wire.prices.gpt.input", &json!(1), false));
    }

    #[test]
    fn an_object_key_inside_an_array_is_sensitive() {
        assert!(redacts("web.font", &json!([{"/Users/x": 1}]), false));
        assert!(redacts("web.font", &json!([{"a": {"~/y": 1}}]), false));
    }

    #[test]
    fn windows_and_variable_paths_are_sensitive_in_keys_and_values() {
        for text in [
            r"C:\Users\alice\acme",
            r"\\host\share",
            "D:/x",
            "%USERPROFILE%",
            "$HOME",
        ] {
            assert!(redacts("theme.name", &json!(text), false), "{text}");
            assert!(
                redacts(&format!("wire.prices.{text}.input"), &json!(1), false),
                "{text}"
            );
            assert_eq!(
                mask_key(&format!("wire.prices.{text}.input")),
                "wire.[redacted]",
                "{text}"
            );
        }
    }

    #[test]
    fn colon_and_percent_without_a_path_shape_stay_visible() {
        assert!(!redacts("wire.prices.x:1.input", &json!(1), false));
        assert!(!redacts("chat.name", &json!("x:1"), false));
        assert!(!redacts("chat.name", &json!("C:foo"), false));
        assert!(!redacts("theme.name", &json!("tokyo-night"), false));
        assert!(!redacts("chat.wrap_width", &json!(96), false));
    }

    #[test]
    fn tight_colour_fields_redact_anything_that_is_not_a_colour() {
        for (key, value) in [
            ("mermaid.style.colors.x", json!("ghp_SECRET")),
            ("mermaid.style.colors.x", json!("hunter2 is the password")),
            ("mermaid.style.background", json!("ghp_SECRET")),
            ("mermaid.style.series", json!(["#fff", "ghp_SECRET"])),
            ("theme.status.base_bg", json!("ghp_SECRET")),
            ("theme.diff.add_fg", json!("rgb(1, 2, ghp)")),
            ("web.ansi_dark", json!(["#000000", "ghp_SECRET"])),
            ("web.ansi_light", json!(["#12345"])),
            ("mermaid.style.palette", json!("a b c")),
            ("mermaid.style.curve", json!("ghp_SECRET")),
            ("theme.name", json!("ghp_SECRET")),
        ] {
            assert!(redacts(key, &value, false), "{key} {value}");
        }
    }

    #[test]
    fn colours_names_and_numbers_stay_visible() {
        for (key, value) in [
            ("mermaid.style.colors.accent", json!("#ff00aa")),
            ("mermaid.style.colors.bg", json!("#FFF")),
            ("mermaid.style.colors.bg", json!("#ff00aa80")),
            ("theme.status.mode_idle_bg", json!("dark_green")),
            ("theme.status.store_fg", json!("reset")),
            ("theme.status.base_bg", json!("rgba(1, 2, 3, 0.5)")),
            ("theme.diff.add_fg", json!("hsl(10, 50%, 50%)")),
            ("theme.input.suggestion", json!("236")),
            ("theme.name", json!("tokyo-night")),
            ("theme.name", json!("Solarized (dark)")),
            ("theme.name", json!("base16-ocean.dark")),
            ("theme.no_background", json!(true)),
            ("mermaid.style.series", json!(["#fff", "#000000"])),
            ("mermaid.style.palette", json!("code")),
            ("mermaid.style.curve", json!("linear")),
            ("mermaid.style.font_size", json!(12.5)),
            ("web.ansi_dark", json!(["#0c0c0c", "#c50f1f"])),
            ("chat.wrap_width", json!(96)),
        ] {
            assert!(!redacts(key, &value, false), "{key} {value}");
        }
    }

    #[test]
    fn closed_enum_fields_redact_unknown_words() {
        assert!(!redacts("workers.view", &json!("table"), false));
        assert!(!redacts("workers.view", &json!("list"), false));
        assert!(!redacts("workers.pr_provider", &json!("github"), false));
        assert!(!redacts("workers.pr_provider", &json!("none"), false));
        assert!(redacts("workers.view", &json!("ghp_SECRET"), false));
        assert!(redacts("workers.view", &json!("grid"), false));
        assert!(redacts("workers.pr_provider", &json!("gitlab"), false));
    }

    #[test]
    fn mermaid_config_keeps_numbers_bools_colours_and_short_words() {
        assert!(!redacts(
            "mermaid.style.config",
            &json!({"theme": "dark", "flowchart": {"nodeSpacing": 30, "curve": "basis", "htmlLabels": true, "c": "#ff00aa"}}),
            false
        ));
    }

    #[test]
    fn mermaid_config_redacts_other_strings_at_any_depth() {
        for value in [
            json!({"note": "correct horse battery staple"}),
            json!({"a": {"b": {"c": "ghp_SECRET"}}}),
            json!({"a": ["ok", "hunter2 secret"]}),
            json!({"a": "averyveryverylongwordthatisnotshort"}),
        ] {
            assert!(redacts("mermaid.style.config", &value, false), "{value}");
        }
    }

    #[test]
    fn price_keys_must_be_model_names() {
        for key in [
            "wire.prices.claude-haiku-5-5.input",
            "wire.prices.gpt-4.1.output",
            "wire.prices.x:1.input",
            "wire.prices.claude-opus-4-1-20250805.cache_read",
        ] {
            assert!(!redacts(key, &json!(1), false), "{key}");
        }
        for key in [
            "wire.prices.hunter2 secret.input",
            "wire.prices.a b.input",
            "wire.prices.\"quoted\".input",
            &format!("wire.prices.{}.input", "a".repeat(65)),
        ] {
            assert!(redacts(key, &json!(1), false), "{key}");
            assert_eq!(mask_key(key), "wire.[redacted]", "{key}");
        }
    }

    #[test]
    fn ticket_keys_follow_the_ident_grammar() {
        assert!(!redacts("tickets.linear", &json!(["GUZ", "VP20"]), false));
        assert!(!redacts("tickets.jira", &json!(["KODA"]), false));
        assert!(redacts(
            "tickets.linear",
            &json!(["my secret ticket"]),
            false
        ));
        assert!(redacts("tickets.jira", &json!(["a".repeat(65)]), false));
    }

    fn assert_secret(key: &str, text: &str) {
        assert!(redacts(key, &json!(text), false), "{key} {text}");
    }

    #[test]
    fn bearer_headers_are_redacted_in_loose_fields() {
        assert_secret("chat.name", "Bearer abc");
        assert_secret("changes.diff_args", "Authorization: Bearer xyz");
    }

    #[test]
    fn jwts_are_redacted_in_loose_fields() {
        assert_secret(
            "chat.name",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcDEF",
        );
    }

    #[test]
    fn aws_key_ids_are_redacted_in_loose_fields() {
        assert_secret("chat.name", "AKIAIOSFODNN7EXAMPLE");
        assert_secret("web.font", "ASIAIOSFODNN7EXAMPLE");
    }

    #[test]
    fn slack_tokens_are_redacted_in_loose_fields() {
        assert_secret("chat.name", "xo\x78b-123-abc");
        assert_secret("nap.model", "xo\x78p-1");
    }

    #[test]
    fn sk_keys_are_redacted_in_loose_fields() {
        assert_secret("claude.model", "sk-ant-api03-abc");
        assert_secret("tag.model", "sk-abc");
    }

    #[test]
    fn pem_blocks_are_redacted_in_loose_fields() {
        assert_secret("chat.name", "-----BEGIN PRIVATE KEY-----");
    }

    #[test]
    fn github_tokens_are_redacted_in_a_model_field() {
        for token in [
            "ghp_SECRET",
            "gho_x",
            "ghu_x",
            "ghs_x",
            "ghr_x",
            "github_pat_x",
        ] {
            assert_secret("claude.model", token);
        }
    }

    #[test]
    fn long_base64_runs_are_redacted_in_loose_fields() {
        assert_secret("chat.name", "aGVsbG8gd29ybGQgc2VjcmV0MTI1");
        assert_secret("codex.model", "dGhpcyBpcyBhIHNlY3JldCB0b2tlbjEyMw==");
    }

    #[test]
    fn long_hex_runs_are_redacted_in_loose_fields() {
        assert_secret(
            "chat.name",
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
        );
    }

    #[test]
    fn prefixed_opaque_runs_are_redacted_in_loose_fields() {
        assert_secret("chat.name", "tok_AbCdEf0123456789xyzQ");
        assert_secret("chat.name", "myapp-AbCdEf0123456789");
        assert_secret("agy.model", "token AbCdEf0123456789");
    }

    #[test]
    fn secrets_in_array_items_and_keys_are_redacted() {
        assert!(redacts(
            "changes.diff_args",
            &json!(["--stat", "ghp_SECRET"]),
            false
        ));
        assert!(redacts("wire.prices.ghp_SECRET.input", &json!(1), false));
    }

    #[test]
    fn model_names_and_ordinary_text_stay_visible() {
        for (key, text) in [
            ("claude.model", "claude-opus-4-1-20250805"),
            ("claude.model", "claude-sonnet-4-5"),
            ("codex.model", "gpt-5.1-codex-max"),
            ("claude.effort", "high"),
            ("chat.name", "ask-me"),
            ("chat.name", "task-force"),
            ("chat.name", "x:1"),
            ("web.font", "\"Geist Mono\", monospace"),
            ("changes.pager", "delta"),
            ("changes.diff_args", "--color=always"),
            ("theme.name", "tokyo-night"),
            ("mermaid.style.colors.accent", "#ff00aa"),
        ] {
            assert!(!redacts(key, &json!(text), false), "{key} {text}");
        }
        assert!(!redacts("chat.wrap_width", &json!(96), false));
        assert!(!redacts("chat.wrap_width", &json!(1), false));
    }

    #[test]
    fn unlisted_keys_fail_closed() {
        assert_eq!(listing("novel.setting"), Listing::Unlisted);
        assert!(redacts("novel.setting", &json!(1), false));
    }
}
