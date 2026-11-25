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
    text.starts_with('/')
        || text.starts_with('~')
        || VALUE_MARKERS.iter().any(|marker| text.contains(marker))
}

fn value_is_sensitive(value: &Value) -> bool {
    match value {
        Value::String(text) => text_is_sensitive(text),
        Value::Array(items) => items.iter().any(value_is_sensitive),
        Value::Object(map) => map.values().any(value_is_sensitive),
        _ => false,
    }
}

pub fn redacts(key: &str, value: &Value, from_env: bool) -> bool {
    from_env || listing(key) != Listing::Safe || value_is_sensitive(value)
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
            "chat.permission_mode",
            "ask.permission_mode",
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
    fn unlisted_keys_fail_closed() {
        assert_eq!(listing("novel.setting"), Listing::Unlisted);
        assert!(redacts("novel.setting", &json!(1), false));
    }
}
