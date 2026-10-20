use crate::highlight::ThemeConfig;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use ralph_core::sanitize::strip_controls;
use std::path::Path;

pub const CHAT_DEFAULT_PERMISSION_MODE: &str = "auto";
pub const CHAT_PERMISSION_MODE_ENV: &str = "RALPH_CHAT_PERMISSION_MODE";
pub const ASK_DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";
pub const ASK_PERMISSION_MODE_ENV: &str = "RALPH_ASK_PERMISSION_MODE";

pub fn resolve_permission_mode(
    flag: Option<&str>,
    env: Option<&str>,
    config: Option<&str>,
    default: &str,
) -> String {
    flag.or(env.filter(|value| !value.is_empty()))
        .or(config)
        .unwrap_or(default)
        .to_string()
}

#[derive(Debug, Clone)]
pub struct InvocationConfig {
    pub prompt: String,
    pub timeout_secs: u64,
    pub theme_config: ThemeConfig,
    pub verbose_tools: VerboseToolsConfig,
    pub permission_mode: String,
}

/// Shell-quote a string for embedding in a single-quoted argument.
///
/// Pure function — no I/O.
fn shell_quote(value: &str) -> String {
    let escaped = value.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

/// Quote prompt text after dropping controls, so a stored NUL cannot reach `sh -c`.
fn quote_prompt(value: &str) -> String {
    shell_quote(&strip_controls(value))
}

fn isolation_args(home: Option<&Path>, auto_compact_off: bool) -> String {
    let mut settings = serde_json::Map::new();
    if auto_compact_off {
        settings.insert("autoCompactEnabled".to_string(), false.into());
    }
    if let Some(h) = home {
        settings.insert(
            "claudeMdExcludes".to_string(),
            serde_json::json!([h.join(".claude").join("CLAUDE.md").to_string_lossy()]),
        );
    }
    let settings_arg = (!settings.is_empty()).then(|| {
        let json = serde_json::Value::Object(settings).to_string();
        format!(" --settings {}", shell_quote(&json))
    });
    format!(
        "--setting-sources project,local{}",
        settings_arg.unwrap_or_default()
    )
}

pub fn build_memory_command(prompt: &str, home: Option<&Path>) -> String {
    format!(
        "claude --verbose --model haiku {} --tools '' --mcp-config '{{\"mcpServers\":{{}}}}' --strict-mcp-config --output-format stream-json -p {}",
        isolation_args(home, false),
        quote_prompt(prompt)
    )
}

pub fn build_chat_command(
    system_prompt: &str,
    extra_args: &[String],
    permission_mode: &str,
    home: Option<&Path>,
) -> String {
    let extra = extra_args
        .iter()
        .map(|arg| format!(" {}", shell_quote(arg)))
        .collect::<String>();
    format!(
        "claude --verbose --permission-mode {} {} --append-system-prompt {}{} --input-format stream-json --output-format stream-json -p",
        permission_mode,
        isolation_args(home, true),
        quote_prompt(system_prompt),
        extra
    )
}

pub fn build_command(prompt: &str, permission_mode: &str) -> String {
    format!(
        "claude --verbose --permission-mode {} --output-format stream-json -p {}",
        permission_mode,
        quote_prompt(prompt)
    )
}

pub fn invoke(config: InvocationConfig) -> Result<StreamingSubprocessResult, SubprocessError> {
    invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
        command: build_command(&config.prompt, &config.permission_mode),
        timeout_secs: Some(config.timeout_secs),
        theme_config: config.theme_config,
        session_elapsed_ms: 0,
        verbose_tools: config.verbose_tools,
        session_info: SpinnerSessionInfo::default(),
        pause: None,
        cancel: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_is_plain_ask() {
        let cmd = build_command("hello", "bypassPermissions");
        assert!(cmd.contains("--permission-mode bypassPermissions"));
        assert!(cmd.contains("-p 'hello'"));
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--append-system-prompt"));
        assert!(!cmd.contains("--setting-sources"));
    }

    #[test]
    fn memory_curator_is_haiku_and_isolated_without_tools() {
        let cmd = build_memory_command("merge it's notes", Some(Path::new("/home/u")));
        assert!(cmd.contains("--model haiku"));
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains("claudeMdExcludes"));
        assert!(cmd.contains("/home/u/.claude/CLAUDE.md"));
        assert!(cmd.contains("--tools ''"));
        assert!(cmd.contains("--strict-mcp-config"));
        assert!(cmd.contains("--output-format stream-json -p 'merge it'\"'\"'s notes'"));
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--resume"));
    }

    #[test]
    fn chat_command_is_fresh_and_isolated() {
        let mcp = vec!["--strict-mcp-config".to_string()];
        let cmd = build_chat_command("sys", &mcp, "plan", Some(Path::new("/home/u")));
        assert!(cmd.contains("--permission-mode plan "));
        assert!(!cmd.contains("--resume") && !cmd.contains("--agent"));
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains("--append-system-prompt 'sys' '--strict-mcp-config'"));
        assert!(cmd.contains("--input-format stream-json --output-format stream-json"));
        assert!(cmd.ends_with(" -p"));
        assert!(cmd.contains(r#"--settings '{"autoCompactEnabled":false,"#));
    }

    #[test]
    fn chat_command_passes_agents_and_keeps_isolation() {
        let cmd = build_chat_command(
            "sys",
            &ralph_core::chat::build_agents_args(),
            "auto",
            Some(Path::new("/home/u")),
        );
        assert!(cmd.contains("'--agents'"));
        assert!(cmd.contains("--setting-sources project,local"));
    }

    #[test]
    fn build_command_escapes_single_quotes() {
        let cmd = build_command("it's a test", "default");
        assert!(cmd.contains("'\"'\"'"));
    }

    #[test]
    fn chat_command_strips_a_stored_nul_before_spawn() {
        let cmd = build_chat_command("sys\0", &[], "auto", None);
        assert!(std::ffi::CString::new(cmd.clone()).is_ok());
        assert!(cmd.contains("--append-system-prompt 'sys'"));
        assert!(!cmd.contains('\u{0001}'));
    }
}
