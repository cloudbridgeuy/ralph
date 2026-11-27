use super::redact_shapes::{key_class, key_shape_ok, text_has_path_shape, value_shape_ok};
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
    "chat.view_pager",
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
    "chat.agy.effort",
    "chat.agy.model",
    "chat.claude.model",
    "chat.emphasis_icons",
    "chat.harness",
    "chat.emphasis_round",
    "chat.empty_submit",
    "chat.hold_progress",
    "chat.inbox_alert_notification",
    "chat.inbox_alert_sound",
    "chat.inbox_alert_unknown_focus",
    "chat.keepalive_hours",
    "chat.program_resume",
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
    "chat.view",
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

const DECLARED_NESTED_KEYS: &[&str] =
    &["chat.claude.permission_mode", "ask.claude.permission_mode"];

const VALUE_MARKERS: [&str; 5] = ["@", "key=", "token=", "password=", "secret="];

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
    let lowered = text.to_ascii_lowercase();
    text_has_path_shape(text) || VALUE_MARKERS.iter().any(|marker| lowered.contains(marker))
}

fn has_free_key_tail(key: &str) -> bool {
    listing(key) != Listing::Safe
        && key.split('.').count() > 2
        && !DECLARED_NESTED_KEYS.contains(&key)
}

fn key_is_sensitive(key: &str) -> bool {
    key.split('.').any(text_is_sensitive) || !key_shape_ok(key)
}

fn key_needs_mask(key: &str) -> bool {
    key_is_sensitive(key) || has_free_key_tail(key)
}

pub fn mask_key(key: &str) -> String {
    if !key_needs_mask(key) {
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
mod tests;
