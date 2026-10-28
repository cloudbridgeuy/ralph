use crate::config::ChatSection;
use crate::highlight::ThemeConfig;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use ralph_core::claude_leftovers::{claude_dir, configured_dir};
use ralph_core::sanitize::strip_controls;
use std::path::{Path, PathBuf};

pub const CHAT_DEFAULT_PERMISSION_MODE: &str = "auto";
pub const CHAT_PERMISSION_MODE_ENV: &str = "RALPH_CHAT_PERMISSION_MODE";
pub const ASK_DEFAULT_PERMISSION_MODE: &str = "bypassPermissions";
pub const ASK_PERMISSION_MODE_ENV: &str = "RALPH_ASK_PERMISSION_MODE";
const CHAT_DISABLE_BACKGROUND_TASKS_ENV: &str = "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1";

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
    pub config_dir: Option<PathBuf>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Isolation {
    pub user_settings: bool,
    pub user_claude_md: bool,
    pub project_excludes: Vec<PathBuf>,
    pub config_dir: Option<PathBuf>,
}

impl Isolation {
    pub fn for_chat(chat: &ChatSection, cwd: &Path, config_dir: Option<&Path>) -> Self {
        Self {
            user_settings: chat.user_settings,
            user_claude_md: chat.user_claude_md,
            project_excludes: if chat.project_claude_md {
                Vec::new()
            } else {
                vec![
                    cwd.join("CLAUDE.md"),
                    cwd.join("CLAUDE.local.md"),
                    cwd.join(".claude").join("CLAUDE.md"),
                ]
            },
            config_dir: config_dir.map(Path::to_path_buf),
        }
    }
}

fn isolation_args(home: Option<&Path>, auto_compact_off: bool, isolation: &Isolation) -> String {
    let mut settings = serde_json::Map::new();
    if auto_compact_off {
        settings.insert("autoCompactEnabled".to_string(), false.into());
    }
    let excludes: Vec<String> = home
        .filter(|_| !isolation.user_claude_md)
        .map(|h| claude_dir(isolation.config_dir.as_deref(), h).join("CLAUDE.md"))
        .iter()
        .chain(&isolation.project_excludes)
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    if !excludes.is_empty() {
        settings.insert("claudeMdExcludes".to_string(), excludes.into());
    }
    let settings_arg = (!settings.is_empty()).then(|| {
        let json = serde_json::Value::Object(settings).to_string();
        format!(" --settings {}", shell_quote(&json))
    });
    let sources = if isolation.user_settings {
        "user,project,local"
    } else {
        "project,local"
    };
    format!(
        "--setting-sources {sources}{}",
        settings_arg.unwrap_or_default()
    )
}

fn config_dir_env(config_dir: Option<&Path>, home: Option<&Path>) -> String {
    configured_dir(config_dir, home)
        .map(|dir| format!("CLAUDE_CONFIG_DIR={} ", shell_quote(&dir.to_string_lossy())))
        .unwrap_or_default()
}

pub fn build_memory_command(
    prompt: &str,
    home: Option<&Path>,
    config_dir: Option<&Path>,
) -> String {
    let isolation = Isolation {
        config_dir: config_dir.map(Path::to_path_buf),
        ..Isolation::default()
    };
    format!(
        "{}claude --verbose --model haiku {} --tools '' --mcp-config '{{\"mcpServers\":{{}}}}' --strict-mcp-config --output-format stream-json -p {}",
        config_dir_env(config_dir, home),
        isolation_args(home, false, &isolation),
        quote_prompt(prompt)
    )
}

pub fn build_chat_command(
    system_prompt: &str,
    extra_args: &[String],
    permission_mode: &str,
    home: Option<&Path>,
    isolation: &Isolation,
) -> String {
    let extra = extra_args
        .iter()
        .map(|arg| format!(" {}", shell_quote(arg)))
        .collect::<String>();
    format!(
        "{} {}claude --verbose --permission-mode {} {} --append-system-prompt {}{} --input-format stream-json --output-format stream-json -p",
        CHAT_DISABLE_BACKGROUND_TASKS_ENV,
        config_dir_env(isolation.config_dir.as_deref(), home),
        permission_mode,
        isolation_args(home, true, isolation),
        quote_prompt(system_prompt),
        extra
    )
}

pub fn build_command(
    prompt: &str,
    permission_mode: &str,
    config_dir: Option<&Path>,
    home: Option<&Path>,
) -> String {
    format!(
        "{}claude --verbose --permission-mode {} --output-format stream-json -p {}",
        config_dir_env(config_dir, home),
        permission_mode,
        quote_prompt(prompt)
    )
}

pub fn invoke(config: InvocationConfig) -> Result<StreamingSubprocessResult, SubprocessError> {
    invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
        command: build_command(
            &config.prompt,
            &config.permission_mode,
            config.config_dir.as_deref(),
            dirs::home_dir().as_deref(),
        ),
        timeout_secs: Some(config.timeout_secs),
        theme_config: config.theme_config,
        session_elapsed_ms: 0,
        verbose_tools: config.verbose_tools,
        session_info: SpinnerSessionInfo::default(),
        gate: None,
        cancel: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_command_is_plain_ask() {
        let cmd = build_command("hello", "bypassPermissions", None, None);
        assert!(cmd.contains("--permission-mode bypassPermissions"));
        assert!(cmd.contains("-p 'hello'"));
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--append-system-prompt"));
        assert!(!cmd.contains("--setting-sources"));
    }

    #[test]
    fn memory_curator_is_haiku_and_isolated_without_tools() {
        let cmd = build_memory_command("merge it's notes", Some(Path::new("/home/u")), None);
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
        let cmd = build_chat_command(
            "sys",
            &mcp,
            "plan",
            Some(Path::new("/home/u")),
            &Isolation::default(),
        );
        assert!(cmd.starts_with("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 claude "));
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
            &Isolation::default(),
        );
        assert!(cmd.contains("'--agents'"));
        assert!(cmd.contains("--setting-sources project,local"));
    }

    #[test]
    fn build_command_escapes_single_quotes() {
        let cmd = build_command("it's a test", "default", None, None);
        assert!(cmd.contains("'\"'\"'"));
    }

    #[test]
    fn chat_command_strips_a_stored_nul_before_spawn() {
        let cmd = build_chat_command("sys\0", &[], "auto", None, &Isolation::default());
        assert!(std::ffi::CString::new(cmd.clone()).is_ok());
        assert!(cmd.contains("--append-system-prompt 'sys'"));
        assert!(!cmd.contains('\u{0001}'));
    }

    #[test]
    fn unset_config_dir_leaves_every_command_as_before() {
        assert_eq!(
            build_command("hi", "auto", None, Some(Path::new("/home/u"))),
            "claude --verbose --permission-mode auto --output-format stream-json -p 'hi'"
        );
        assert_eq!(
            build_memory_command("hi", Some(Path::new("/home/u")), None),
            r#"claude --verbose --model haiku --setting-sources project,local --settings '{"claudeMdExcludes":["/home/u/.claude/CLAUDE.md"]}' --tools '' --mcp-config '{"mcpServers":{}}' --strict-mcp-config --output-format stream-json -p 'hi'"#
        );
        assert_eq!(
            build_chat_command("sys", &[], "auto", None, &Isolation::default()),
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 claude --verbose --permission-mode auto --setting-sources project,local --settings '{\"autoCompactEnabled\":false}' --append-system-prompt 'sys' --input-format stream-json --output-format stream-json -p"
        );
    }

    #[test]
    fn config_dir_prefixes_every_command_and_moves_the_exclude() {
        let home = Some(Path::new("/home/u"));
        let dir = Some(Path::new("~/alt dir"));
        let ask = build_command("hi", "auto", dir, home);
        assert!(ask.starts_with("CLAUDE_CONFIG_DIR='/home/u/alt dir' claude --verbose "));
        let memory = build_memory_command("hi", home, dir);
        assert!(memory.starts_with("CLAUDE_CONFIG_DIR='/home/u/alt dir' claude "));
        assert!(memory.contains(r#"["/home/u/alt dir/CLAUDE.md"]"#));
        let isolation = Isolation::for_chat(&ChatSection::default(), Path::new("/work"), dir);
        let chat = build_chat_command("sys", &[], "auto", home, &isolation);
        assert!(chat.starts_with(
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 CLAUDE_CONFIG_DIR='/home/u/alt dir' claude "
        ));
        assert!(chat.contains(r#""claudeMdExcludes":["/home/u/alt dir/CLAUDE.md"]"#));
        assert!(!chat.contains("/.claude/CLAUDE.md"));
    }

    fn chat_args(chat: ChatSection) -> String {
        isolation_args(
            Some(Path::new("/home/u")),
            true,
            &Isolation::for_chat(&chat, Path::new("/work"), None),
        )
    }

    #[test]
    fn default_chat_isolation_is_unchanged() {
        assert_eq!(
            chat_args(ChatSection::default()),
            r#"--setting-sources project,local --settings '{"autoCompactEnabled":false,"claudeMdExcludes":["/home/u/.claude/CLAUDE.md"]}'"#
        );
    }

    #[test]
    fn user_settings_adds_the_user_source_only() {
        let args = chat_args(ChatSection {
            user_settings: true,
            ..ChatSection::default()
        });
        assert!(args.starts_with("--setting-sources user,project,local "));
        assert!(args.contains(r#""claudeMdExcludes":["/home/u/.claude/CLAUDE.md"]"#));
    }

    #[test]
    fn user_claude_md_drops_only_the_home_exclude() {
        let args = chat_args(ChatSection {
            user_claude_md: true,
            ..ChatSection::default()
        });
        assert_eq!(
            args,
            r#"--setting-sources project,local --settings '{"autoCompactEnabled":false}'"#
        );
    }

    #[test]
    fn project_claude_md_off_excludes_the_cwd_memory_files() {
        let args = chat_args(ChatSection {
            project_claude_md: false,
            ..ChatSection::default()
        });
        assert!(args.contains(
            r#""claudeMdExcludes":["/home/u/.claude/CLAUDE.md","/work/CLAUDE.md","/work/CLAUDE.local.md","/work/.claude/CLAUDE.md"]"#
        ));
    }

    #[test]
    fn all_switches_together() {
        let args = chat_args(ChatSection {
            user_settings: true,
            user_claude_md: true,
            project_claude_md: false,
            ..ChatSection::default()
        });
        assert_eq!(
            args,
            r#"--setting-sources user,project,local --settings '{"autoCompactEnabled":false,"claudeMdExcludes":["/work/CLAUDE.md","/work/CLAUDE.local.md","/work/.claude/CLAUDE.md"]}'"#
        );
    }
}
