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
    include_str!("../../config.rs"),
    include_str!("../agy.rs"),
    include_str!("../changes.rs"),
    include_str!("../claude.rs"),
    include_str!("../codex.rs"),
    include_str!("../cursor.rs"),
    include_str!("../opencode.rs"),
    include_str!("../usage.rs"),
    include_str!("../web.rs"),
    include_str!("../wire.rs"),
    include_str!("../workers.rs"),
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
fn free_text_map_keys_redact_the_row_and_mask_the_key() {
    for key in [
        "mermaid.style.colors.hunter2 secret",
        "theme.status.my password is hunter2",
        "mermaid.style.config.flowchart.correct horse battery staple",
    ] {
        let value = if key.contains("flowchart") {
            json!(30)
        } else {
            json!("#fff")
        };
        assert!(redacts(key, &value, false), "{key}");
        let masked = mask_key(key);
        assert!(masked.ends_with(".[redacted]"), "{masked}");
        assert!(
            !masked.contains("secret") && !masked.contains("horse"),
            "{masked}"
        );
    }
}

#[test]
fn real_theme_and_mermaid_keys_stay_visible() {
    for key in [
        "theme.status.base_bg",
        "mermaid.style.colors.node",
        "mermaid.style.colors.edge",
        "mermaid.style.font_size",
        "mermaid.style.config.flowchart.nodeSpacing",
    ] {
        assert_eq!(mask_key(key), key);
    }
    assert!(!redacts(
        "mermaid.style.config.flowchart.nodeSpacing",
        &json!(30),
        false
    ));
}

#[test]
fn user_chosen_keys_below_non_safe_rows_are_masked() {
    for key in [
        "chat.reset_turns.secret words",
        "chat.editor.secret words",
        "workers.shell.secret words",
        "claude.permissions.secret words",
        "codex.sandbox.secret",
        "novel.setting.x",
    ] {
        assert_eq!(
            mask_key(key),
            format!("{}.[redacted]", key.split('.').next().unwrap()),
            "{key}"
        );
        assert!(redacts(key, &json!(1), false), "{key}");
    }
}

#[test]
fn declared_non_safe_field_names_stay_visible() {
    for key in [
        "claude.permissions",
        "codex.sandbox",
        "chat.editor",
        "workers.shell",
        "chat.claude.permission_mode",
        "ask.claude.permission_mode",
        "novel.setting",
    ] {
        assert_eq!(mask_key(key), key, "{key}");
    }
}

#[test]
fn object_keys_inside_a_mermaid_config_array_follow_the_key_grammar() {
    assert!(redacts(
        "mermaid.style.config",
        &json!({"x": [{"correct horse battery": 1}]}),
        false
    ));
    assert!(!redacts(
        "mermaid.style.config",
        &json!({"x": [{"nodeSpacing": 1}]}),
        false
    ));
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

const OPAQUE_BODY: &str = "AbCdEf0123456789AbCdEf0123456789abcd";

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
    assert_secret(
        "chat.name",
        "xo\x78b-123456789012-1234567890123-AbCdEf0123456789AbCdEf01",
    );
    assert_secret("nap.model", &format!("xo\x78p-1-2-3-{OPAQUE_BODY}"));
}

#[test]
fn sk_keys_are_redacted_in_loose_fields() {
    assert_secret("claude.model", &format!("sk-ant-api03-{OPAQUE_BODY}"));
    assert_secret("tag.model", &format!("sk-{OPAQUE_BODY}"));
}

#[test]
fn pem_blocks_are_redacted_in_loose_fields() {
    assert_secret("chat.name", "-----BEGIN PRIVATE KEY-----");
}

#[test]
fn github_tokens_are_redacted_in_a_model_field() {
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
        assert_secret("claude.model", &format!("{prefix}{OPAQUE_BODY}"));
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
fn punctuated_and_split_secrets_are_redacted_in_loose_fields() {
    for text in [
        "pass:AbCdEf0123456789xyzQ",
        "AbCdEf0123456789xyzQ.",
        "Zx9k2Lq-8vB3mN0pQrS-7tUwXyZ1aB2cD3e",
        "550e8400-e29b-41d4-a716-446655440000",
    ] {
        assert_secret("chat.name", text);
    }
    assert_secret("theme.name", "AbCdEf0123.456789xyzQAbCd");
    assert_secret("tickets.linear", "AbCdEf0123.456789xyzQAbCd");
}

#[test]
fn a_dotted_opaque_price_key_is_redacted_and_masked() {
    let key = "wire.prices.AbCdEf0123456.789xyzQAbCd.input";
    assert!(redacts(key, &json!(1), false));
    assert_eq!(mask_key(key), "wire.[redacted]");
    assert!(!redacts("wire.prices.gpt-4.1-mini.input", &json!(1), false));
}

#[test]
fn secrets_in_array_items_and_keys_are_redacted() {
    assert!(redacts(
        "changes.diff_args",
        &json!(["--stat", format!("ghp_{OPAQUE_BODY}")]),
        false
    ));
    assert!(redacts(
        &format!("wire.prices.ghp_{OPAQUE_BODY}.input"),
        &json!(1),
        false
    ));
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

const NUMERIC_SETTINGS: [&str; 3] = [
    "chat.reset_turns",
    "chat.reset_tokens",
    "chat.worker_quiet_secs",
];

#[test]
fn numeric_settings_redact_text_arrays_and_tables() {
    for key in NUMERIC_SETTINGS {
        for value in [
            json!("my password is hunter2"),
            json!("hunter2"),
            json!(["my secret words"]),
            json!({"hunter2": 1}),
            json!(true),
        ] {
            assert!(redacts(key, &value, false), "{key} {value}");
        }
    }
}

#[test]
fn numeric_settings_keep_numbers_visible() {
    for key in NUMERIC_SETTINGS {
        for value in [json!(40), json!(250_000), json!(2.5)] {
            assert!(!redacts(key, &value, false), "{key} {value}");
        }
    }
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
fn loose_fields_redact_punctuated_and_dashed_secrets() {
    for text in PUNCTUATED_SECRETS {
        for key in ["chat.name", "claude.model", "nap.harness", "agy.effort"] {
            assert_secret(key, text);
        }
        assert!(
            redacts("tickets.linear", &json!([text]), false),
            "tickets.linear {text}"
        );
    }
}

#[test]
fn loose_array_items_are_checked_one_by_one() {
    assert!(redacts(
        "changes.diff_args",
        &json!(["--stat", "AbCdEf0123456789xyzQ!"]),
        false
    ));
    assert!(redacts(
        "mermaid.style.font_family",
        &json!(["Inter", "x*AbCdEf0123456789xyzQ"]),
        false
    ));
}

#[test]
fn loose_fields_redact_long_pieces_and_long_values() {
    assert_secret("chat.name", &"a".repeat(25));
    assert_secret("chat.name", &format!("x-{}", "b".repeat(25)));
    assert_secret("claude.model", &["xyz"; 17].join("-"));
    assert!(!redacts("chat.name", &json!("a".repeat(24)), false));
}

#[test]
fn loose_fields_redact_long_hex_runs_split_by_separators() {
    assert_secret("chat.name", "deadbeef-cafebabe-deadbeef-cafebabe");
    assert_secret("chat.name", "id-deadbeef-cafebabe-deadbeef-cafebabe");
    assert!(!redacts("chat.name", &json!("deadbeef-cafebabe"), false));
    assert!(!redacts(
        "chat.name",
        &json!("2024-08-06-2024-08-06"),
        false
    ));
}

#[test]
fn loose_fields_redact_long_letter_digit_pieces() {
    assert_secret("chat.name", "AbCdEf0123");
    assert_secret("chat.name", "ghp-AbCdEf0123");
    assert!(!redacts("chat.name", &json!("gpt4o-v2"), false));
}

#[test]
fn spaces_and_quotes_are_hidden_outside_font_fields() {
    for (key, text) in [
        ("chat.name", "my secret words"),
        ("claude.model", "a b"),
        ("changes.pager", "delta --side-by-side"),
        ("changes.diff_args", "--stat --color"),
        ("chat.worker_base", "\"main\""),
        ("nap.model", "a,b"),
        ("chat.name", "a=b"),
    ] {
        assert_secret(key, text);
    }
}

#[test]
fn font_fields_allow_names_quotes_commas_and_spaces_only() {
    for key in [
        "web.font",
        "mermaid.style.font_family",
        "mermaid.style.font_family_mono",
    ] {
        assert!(
            !redacts(
                key,
                &json!("\"Geist Mono\", ui-monospace, Menlo, monospace"),
                false
            ),
            "{key}"
        );
        assert!(!redacts(key, &json!("Helvetica Neue"), false), "{key}");
        assert!(redacts(key, &json!("Mono!"), false), "{key}");
        assert!(redacts(key, &json!("\"Geist Mono\"; x"), false), "{key}");
        assert!(redacts(key, &json!("a=b"), false), "{key}");
    }
    assert_secret("web.font", "\"AbCdEf0123456789xyzQ\", monospace");
    assert_secret("web.font", &"Mono, ".repeat(25));
}

#[test]
fn text_is_sensitive_flags_each_marker_and_path_shape() {
    for text in [
        "https://x",
        "a@b",
        "key=1",
        "token=1",
        "password=1",
        "secret=1",
        "KEY=1",
        "Token=1",
        "PASSWORD=1",
        "SECRET=1",
        "~",
        "a~b",
        "/x",
        r"a\b",
        "%A%",
        "$A",
    ] {
        assert!(text_is_sensitive(text), "{text}");
    }
    for text in ["tokyo-night", "--color=always", "x:1", "monkey", "secrets"] {
        assert!(!text_is_sensitive(text), "{text}");
    }
}

#[test]
fn value_is_sensitive_walks_strings_arrays_and_object_keys() {
    for value in [
        json!("a@b"),
        json!(["a", "a@b"]),
        json!({"a@b": 1}),
        json!({"k": "a@b"}),
        json!({"k": [{"j": "~"}]}),
    ] {
        assert!(value_is_sensitive(&value), "{value}");
    }
    for value in [json!("a"), json!(1), json!(true), json!({"k": ["v", 2]})] {
        assert!(!value_is_sensitive(&value), "{value}");
    }
}

#[test]
fn secret_markers_match_in_any_case_in_flag_arguments() {
    for text in [
        "GH_TOKEN=abc123",
        "API_KEY=hunter2",
        "PASSWORD=hunter2",
        "SECRET=x",
        "--password=hunter2",
        "--secret=x",
        "Token=abc",
        "--api-key=abc",
    ] {
        assert!(
            redacts("changes.diff_args", &json!([text]), false),
            "{text}"
        );
    }
    assert!(!redacts(
        "changes.diff_args",
        &json!(["--word-diff=color", "--diff-algorithm=patience"]),
        false
    ));
}

#[test]
fn diff_args_allow_equals_signs_in_flags() {
    for text in [
        "--stat",
        "-U3",
        "--color=always",
        "--diff-algorithm=patience",
    ] {
        assert!(
            !redacts("changes.diff_args", &json!([text]), false),
            "{text}"
        );
    }
    assert_secret("changes.diff_args", "--token=AbCdEf0123456789xyzQ");
    assert_secret("changes.diff_args", "--git-dir=/Users/me");
}

#[test]
fn real_model_and_setting_names_stay_visible() {
    for (key, text) in [
        ("claude.model", "claude-opus-4-1-20250805"),
        ("claude.model", "claude-3-5-sonnet-20241022"),
        ("codex.model", "gpt-4o-2024-08-06"),
        ("codex.model", "gpt-4.1-mini"),
        ("codex.model", "o3-mini-2025-01-31"),
        ("nap.model", "anthropic.claude-sonnet-4-20250514-v1:0"),
        ("agy.model", "gemini-2.0-flash-001"),
        ("chat.name", "ask-me"),
        ("chat.name", "task-force"),
        ("chat.name", "x:1"),
        ("chat.name", "base16-ocean.dark"),
        ("chat.name", "tokyo-night"),
        ("chat.edit_mode", "vi"),
        ("changes.pager", "delta"),
    ] {
        assert!(!redacts(key, &json!(text), false), "{key} {text}");
    }
}

#[test]
fn default_config_rows_are_not_redacted_by_shape() {
    let table = Table::try_from(AppConfig::new()).unwrap();
    let mut keys = Vec::new();
    leaf_keys(&table, "", &mut keys);
    let defaults = serde_json::to_value(&table).unwrap();
    let mut hidden = Vec::new();
    for key in keys {
        let value = key
            .split('.')
            .try_fold(&defaults, |node, part| node.get(part))
            .cloned()
            .unwrap_or_default();
        if listing(&key) == Listing::Safe && redacts(&key, &value, false) {
            hidden.push(key);
        }
    }
    assert!(hidden.is_empty(), "default rows hidden: {hidden:?}");
}

#[test]
fn unlisted_keys_fail_closed() {
    assert_eq!(listing("novel.setting"), Listing::Unlisted);
    assert!(redacts("novel.setting", &json!(1), false));
}

const KEY_GHP: &str = "ghp_AbCdEf0123456789AbCdEf0123456789abcd";
const KEY_HEX: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
const KEY_LETTERS: &str = "QWERTYUIOPASDFGHJKLZXCVBNMQWERTYUIOPASDF";

#[test]
fn opaque_row_keys_are_redacted_and_masked() {
    for key in [
        format!("theme.status.{KEY_GHP}"),
        format!("mermaid.style.colors.{KEY_HEX}"),
        format!("mermaid.style.config.x.{KEY_HEX}.{KEY_HEX}"),
        format!("theme.status.{KEY_LETTERS}"),
    ] {
        assert!(redacts(&key, &json!("red"), false), "{key}");
        let masked = mask_key(&key);
        assert!(masked.ends_with(REDACTED), "{key}");
        assert!(
            !masked.contains("AbCdEf") && !masked.contains("9f86"),
            "{key}"
        );
    }
}

#[test]
fn opaque_object_keys_in_arrays_and_tables_are_redacted() {
    for value in [
        json!([{ KEY_GHP: 1 }]),
        json!({"a": [{"b": { KEY_HEX: 1 }}]}),
        json!([{ KEY_LETTERS: "red" }]),
    ] {
        assert!(
            redacts("mermaid.style.config.arr", &value, false),
            "{value}"
        );
    }
    assert!(redacts(
        "mermaid.style.config.arr",
        &json!([{ KEY_GHP: 1 }]),
        false
    ));
}

#[test]
fn real_object_keys_in_arrays_stay_visible() {
    assert!(!redacts(
        "mermaid.style.config.arr",
        &json!([{"flowchart": {"font_size": 14, "curve": "basis"}}]),
        false
    ));
}

#[test]
fn code_theme_names_stay_visible() {
    for (key, text) in [
        ("theme.assistant.code", "Monokai Extended"),
        ("theme.agent.code", "Solarized (dark)"),
        ("theme.agent", "Solarized (dark)"),
    ] {
        assert!(!redacts(key, &json!(text), false), "{key} {text}");
    }
}

#[test]
fn secrets_in_code_theme_fields_stay_redacted() {
    for key in ["theme.assistant.code", "theme.agent.code", "theme.agent"] {
        assert!(redacts(key, &json!(KEY_GHP), false), "{key}");
        assert!(
            redacts(key, &json!("AbCdEf0123456789AbCdEf012345678"), false),
            "{key}"
        );
    }
}

#[test]
fn opaque_ticket_keys_are_redacted_and_masked() {
    for key in [
        format!("tickets.{KEY_GHP}"),
        "tickets.my secret words".to_string(),
    ] {
        assert!(redacts(&key, &json!(["GUZ"]), false), "{key}");
        assert_eq!(mask_key(&key), "tickets.[redacted]", "{key}");
    }
    assert_eq!(mask_key("tickets.linear"), "tickets.linear");
}
