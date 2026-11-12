use crate::highlight::ThemeConfig;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{
    invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use ralph_core::claude_leftovers::{claude_dir, configured_dir};
use ralph_core::sanitize::strip_controls;
use ralph_core::worker::inbox_env;
use ralph_server::config::ChatSection;
use std::io;
use std::path::{Path, PathBuf};

pub const CHAT_PERMISSION_MODE_ENV: &str = "RALPH_CHAT_PERMISSION_MODE";
pub const ASK_PERMISSION_MODE_ENV: &str = "RALPH_ASK_PERMISSION_MODE";
const CHAT_DISABLE_BACKGROUND_TASKS_ENV: &str = "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1";

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
    settings.insert("totalTokensReminder".to_string(), "off".into());
    let json = serde_json::Value::Object(settings).to_string();
    let settings_arg = format!(" --settings {}", shell_quote(&json));
    let sources = if isolation.user_settings {
        "user,project,local"
    } else {
        "project,local"
    };
    format!("--setting-sources {sources}{settings_arg}")
}

fn config_dir_env(config_dir: Option<&Path>, home: Option<&Path>) -> String {
    configured_dir(config_dir, home)
        .map(|dir| format!("CLAUDE_CONFIG_DIR={} ", shell_quote(&dir.to_string_lossy())))
        .unwrap_or_default()
}

pub fn prompt_stdin(prompt: &str) -> String {
    strip_controls(prompt).into_owned()
}

pub fn build_memory_command(home: Option<&Path>, config_dir: Option<&Path>) -> String {
    memory_command("haiku", "", home, config_dir)
}

pub fn build_nap_command(home: Option<&Path>, config_dir: Option<&Path>) -> String {
    build_tag_command(
        "haiku",
        &ralph_core::chat_memory::nap_system(),
        home,
        config_dir,
    )
}

/// The tag pass: `model` with the labelling prompt as the whole system prompt.
pub fn build_tag_command(
    model: &str,
    system_prompt: &str,
    home: Option<&Path>,
    config_dir: Option<&Path>,
) -> String {
    let system = format!("--system-prompt {} ", shell_quote(system_prompt));
    memory_command(model, &system, home, config_dir)
}

fn memory_command(
    model: &str,
    system: &str,
    home: Option<&Path>,
    config_dir: Option<&Path>,
) -> String {
    let isolation = Isolation {
        config_dir: config_dir.map(Path::to_path_buf),
        ..Isolation::default()
    };
    format!(
        "{}claude --verbose --model {model} {system}{} --tools '' --mcp-config '{{\"mcpServers\":{{}}}}' --strict-mcp-config --output-format stream-json -p",
        config_dir_env(config_dir, home),
        isolation_args(home, false, &isolation),
    )
}

pub fn build_chat_command(
    system_prompt_file: &Path,
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
        "{} {}claude --verbose --permission-mode {} {} --append-system-prompt-file {}{} --input-format stream-json --output-format stream-json -p",
        CHAT_DISABLE_BACKGROUND_TASKS_ENV,
        config_dir_env(isolation.config_dir.as_deref(), home),
        permission_mode,
        isolation_args(home, true, isolation),
        shell_quote(&system_prompt_file.to_string_lossy()),
        extra
    )
}

pub fn with_chat_inbox(command: String, store: &Path) -> io::Result<String> {
    let (key, inbox) = inbox_env(&std::fs::canonicalize(store)?);
    Ok(format!(
        "{key}={} {command}",
        shell_quote(&inbox.to_string_lossy())
    ))
}

pub fn build_command(
    permission_mode: &str,
    config_dir: Option<&Path>,
    home: Option<&Path>,
) -> String {
    format!(
        "{}claude --verbose --permission-mode {} --output-format stream-json -p",
        config_dir_env(config_dir, home),
        permission_mode,
    )
}

pub fn invoke(config: InvocationConfig) -> Result<StreamingSubprocessResult, SubprocessError> {
    invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
        command: build_command(
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
        stdin: Some(prompt_stdin(&config.prompt)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_inbox_command_uses_the_canonical_store_and_quotes_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("chat's store");
        std::fs::create_dir(&store).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&store, &alias).unwrap();
        let canonical = std::fs::canonicalize(&store).unwrap().join("inbox");
        assert_eq!(
            with_chat_inbox("claude -p".to_string(), &alias).unwrap(),
            format!(
                "RALPH_INBOX={} claude -p",
                shell_quote(&canonical.to_string_lossy())
            )
        );
    }

    #[test]
    fn chat_inbox_command_rejects_a_missing_store() {
        let dir = tempfile::tempdir().unwrap();
        assert!(with_chat_inbox("claude -p".to_string(), &dir.path().join("missing")).is_err());
    }

    #[test]
    fn build_command_is_plain_ask() {
        let cmd = build_command("bypassPermissions", None, None);
        assert!(cmd.contains("--permission-mode bypassPermissions"));
        assert!(cmd.ends_with(" -p"), "{cmd}");
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--append-system-prompt"));
        assert!(!cmd.contains("--setting-sources"));
    }

    #[test]
    fn tagger_is_haiku_5_5_with_the_prompt_as_the_system_prompt_and_no_tools() {
        let cmd = build_tag_command(
            "claude-haiku-5-5",
            "Label 'x'.",
            Some(Path::new("/home/u")),
            None,
        );
        assert!(cmd.contains("--model claude-haiku-5-5 "), "{cmd}");
        assert!(
            cmd.contains(r#"--system-prompt 'Label '"'"'x'"'"'.'"#),
            "{cmd}"
        );
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains("--tools ''"));
        assert!(cmd.contains("--mcp-config '{\"mcpServers\":{}}' --strict-mcp-config"));
        assert!(cmd.ends_with("--output-format stream-json -p"), "{cmd}");
        assert!(!cmd.contains("--resume"));
    }

    #[test]
    fn tagger_uses_the_model_it_is_given() {
        let cmd = build_tag_command("model-x", "p", None, None);
        assert!(cmd.contains("--model model-x "), "{cmd}");
        assert!(!cmd.contains("haiku"), "{cmd}");
    }

    #[test]
    fn nap_curator_gets_the_nap_rules_as_its_system_prompt() {
        let cmd = build_nap_command(Some(Path::new("/home/u")), None);
        let system = ralph_core::chat_memory::nap_system();
        assert!(
            cmd.contains(&format!("--system-prompt {} ", shell_quote(&system))),
            "{cmd}"
        );
        assert!(cmd.contains("Invent nothing"), "{cmd}");
        assert!(cmd.contains("--model haiku "), "{cmd}");
        assert!(cmd.contains("--tools ''"), "{cmd}");
        assert!(cmd.ends_with("--output-format stream-json -p"), "{cmd}");
    }

    #[test]
    fn nap_curator_drops_every_claude_md_and_auto_memory_but_merge_and_tag_keep_them() {
        let home = Some(Path::new("/home/u"));
        let nap = build_nap_command(home, None);
        for needle in [
            r#""autoMemoryEnabled":false"#,
            r#""**/CLAUDE.md""#,
            r#""**/CLAUDE.local.md""#,
        ] {
            assert!(nap.contains(needle), "{needle}: {nap}");
        }
        assert_eq!(nap.matches("--settings ").count(), 1, "{nap}");
        for other in [
            build_memory_command(home, None),
            build_tag_command("claude-haiku-5-5", "p", home, None),
        ] {
            assert!(!other.contains("autoMemoryEnabled"), "{other}");
            assert!(!other.contains("**/CLAUDE.md"), "{other}");
        }
    }

    #[test]
    fn merge_curator_keeps_the_default_system_prompt() {
        let cmd = build_memory_command(Some(Path::new("/home/u")), None);
        assert!(!cmd.contains("--system-prompt"), "{cmd}");
    }

    #[test]
    fn memory_curator_is_haiku_and_isolated_without_tools() {
        let cmd = build_memory_command(Some(Path::new("/home/u")), None);
        assert!(cmd.contains("--model haiku"));
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains("claudeMdExcludes"));
        assert!(cmd.contains("/home/u/.claude/CLAUDE.md"));
        assert!(cmd.contains("--tools ''"));
        assert!(cmd.contains("--strict-mcp-config"));
        assert!(cmd.ends_with("--output-format stream-json -p"), "{cmd}");
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--resume"));
    }

    #[test]
    fn chat_command_is_fresh_and_isolated() {
        let mcp = vec!["--strict-mcp-config".to_string()];
        let cmd = build_chat_command(
            Path::new("/s/chat-1/system-prompt.md"),
            &mcp,
            "plan",
            Some(Path::new("/home/u")),
            &Isolation::default(),
        );
        assert!(cmd.starts_with("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 claude "));
        assert!(cmd.contains("--permission-mode plan "));
        assert!(!cmd.contains("--resume") && !cmd.contains("--agent"));
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains(
            "--append-system-prompt-file '/s/chat-1/system-prompt.md' '--strict-mcp-config'"
        ));
        assert!(cmd.contains("--input-format stream-json --output-format stream-json"));
        assert!(cmd.ends_with(" -p"));
        assert!(cmd.contains(r#"--settings '{"autoCompactEnabled":false,"#));
    }

    #[test]
    fn chat_command_passes_agents_and_keeps_isolation() {
        let cmd = build_chat_command(
            Path::new("/s/p.md"),
            &ralph_core::chat::build_agents_args(),
            "auto",
            Some(Path::new("/home/u")),
            &Isolation::default(),
        );
        assert!(cmd.contains("'--agents'"));
        assert!(cmd.contains("--setting-sources project,local"));
    }

    #[test]
    fn chat_and_nap_turn_off_the_total_tokens_reminder() {
        let chat = build_chat_command(
            Path::new("/s/p.md"),
            &[],
            "auto",
            None,
            &Isolation::default(),
        );
        let nap = build_memory_command(None, None);
        for cmd in [chat, nap] {
            assert!(cmd.contains(r#""totalTokensReminder":"off""#), "{cmd}");
        }
    }

    const SECRET_LIKE_PROMPT: &str = "use KEY=abc123 and it's \"$x\"\nñ";

    #[test]
    fn prompts_stay_off_every_command_line() {
        let home = Some(Path::new("/home/u"));
        let commands = [
            build_command("auto", None, home),
            build_memory_command(home, None),
            build_chat_command(
                Path::new("/s/p.md"),
                &[],
                "auto",
                home,
                &Isolation::default(),
            ),
        ];
        for cmd in commands {
            assert!(!cmd.contains("abc123"), "{cmd}");
            assert!(!cmd.contains("--append-system-prompt '"), "{cmd}");
            assert!(!cmd.contains("-p '"), "{cmd}");
        }
    }

    #[test]
    fn prompt_stdin_keeps_the_bytes_the_argument_carried() {
        assert_eq!(prompt_stdin(SECRET_LIKE_PROMPT), SECRET_LIKE_PROMPT);
        assert_eq!(prompt_stdin("hi\0 there\u{1}"), "hi there");
    }

    #[test]
    fn unset_config_dir_adds_no_env_prefix() {
        assert_eq!(
            build_command("auto", None, Some(Path::new("/home/u"))),
            "claude --verbose --permission-mode auto --output-format stream-json -p"
        );
        assert_eq!(
            build_memory_command(Some(Path::new("/home/u")), None),
            r#"claude --verbose --model haiku --setting-sources project,local --settings '{"claudeMdExcludes":["/home/u/.claude/CLAUDE.md"],"totalTokensReminder":"off"}' --tools '' --mcp-config '{"mcpServers":{}}' --strict-mcp-config --output-format stream-json -p"#
        );
        assert_eq!(
            build_chat_command(Path::new("/s/p.md"), &[], "auto", None, &Isolation::default()),
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 claude --verbose --permission-mode auto --setting-sources project,local --settings '{\"autoCompactEnabled\":false,\"totalTokensReminder\":\"off\"}' --append-system-prompt-file '/s/p.md' --input-format stream-json --output-format stream-json -p"
        );
    }

    #[test]
    fn config_dir_prefixes_every_command_and_moves_the_exclude() {
        let home = Some(Path::new("/home/u"));
        let dir = Some(Path::new("~/alt dir"));
        let ask = build_command("auto", dir, home);
        assert!(ask.starts_with("CLAUDE_CONFIG_DIR='/home/u/alt dir' claude --verbose "));
        let memory = build_memory_command(home, dir);
        assert!(memory.starts_with("CLAUDE_CONFIG_DIR='/home/u/alt dir' claude "));
        assert!(memory.contains(r#"["/home/u/alt dir/CLAUDE.md"]"#));
        let isolation = Isolation::for_chat(&ChatSection::default(), Path::new("/work"), dir);
        let chat = build_chat_command(Path::new("/s/p.md"), &[], "auto", home, &isolation);
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
            r#"--setting-sources project,local --settings '{"autoCompactEnabled":false,"claudeMdExcludes":["/home/u/.claude/CLAUDE.md"],"totalTokensReminder":"off"}'"#
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
            r#"--setting-sources project,local --settings '{"autoCompactEnabled":false,"totalTokensReminder":"off"}'"#
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
            r#"--setting-sources user,project,local --settings '{"autoCompactEnabled":false,"claudeMdExcludes":["/work/CLAUDE.md","/work/CLAUDE.local.md","/work/.claude/CLAUDE.md"],"totalTokensReminder":"off"}'"#
        );
    }
}
