use crate::config::ChatSection;
use crate::subprocess::Invocation;
use ralph_core::claude_leftovers::{claude_dir, configured_dir, CLAUDE_CONFIG_DIR_ENV};
use ralph_core::claude_md::chat_project_excludes;
use ralph_core::sanitize::strip_controls;
use ralph_core::worker::{inbox_env, ClaudeMode, CLAUDE_COMMAND, CODEX_COMMAND};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

pub const CHAT_PERMISSION_MODE_ENV: &str = "RALPH_CHAT_PERMISSION_MODE";
pub const ASK_PERMISSION_MODE_ENV: &str = "RALPH_ASK_PERMISSION_MODE";
const CHAT_DISABLE_BACKGROUND_TASKS_ENV: &str = "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Isolation {
    pub user_settings: bool,
    pub user_claude_md: bool,
    pub project_excludes: Vec<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub no_auto_memory: bool,
}

impl Isolation {
    pub fn for_chat(chat: &ChatSection, cwd: &Path, config_dir: Option<&Path>) -> Self {
        Self {
            user_settings: chat.user_settings,
            user_claude_md: chat.user_claude_md,
            project_excludes: if chat.project_claude_md {
                Vec::new()
            } else {
                chat_project_excludes(cwd)
            },
            config_dir: config_dir.map(Path::to_path_buf),
            no_auto_memory: false,
        }
    }

    fn for_memory(config_dir: Option<&Path>) -> Self {
        Self {
            config_dir: config_dir.map(Path::to_path_buf),
            ..Self::default()
        }
    }

    fn for_nap(config_dir: Option<&Path>) -> Self {
        Self {
            project_excludes: vec![
                PathBuf::from("**/CLAUDE.md"),
                PathBuf::from("**/CLAUDE.local.md"),
            ],
            no_auto_memory: true,
            ..Self::for_memory(config_dir)
        }
    }
}

fn isolation_args(
    home: Option<&Path>,
    auto_compact_off: bool,
    isolation: &Isolation,
    mcptools: bool,
) -> [String; 4] {
    let mut settings = serde_json::Map::new();
    if auto_compact_off {
        settings.insert("autoCompactEnabled".to_string(), false.into());
    }
    if isolation.no_auto_memory {
        settings.insert("autoMemoryEnabled".to_string(), false.into());
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
    if mcptools {
        settings.insert(
            "permissions".to_string(),
            ralph_core::chat::mcptools_ask_settings()["permissions"].clone(),
        );
    }
    settings.insert("totalTokensReminder".to_string(), "off".into());
    let json = serde_json::Value::Object(settings).to_string();
    let sources = if isolation.user_settings {
        "user,project,local"
    } else {
        "project,local"
    };
    [
        "--setting-sources".to_string(),
        sources.to_string(),
        "--settings".to_string(),
        json,
    ]
}

fn config_dir_env(
    config_dir: Option<&Path>,
    home: Option<&Path>,
) -> Option<(&'static str, OsString)> {
    configured_dir(config_dir, home).map(|dir| (CLAUDE_CONFIG_DIR_ENV, dir.into()))
}

pub fn prompt_stdin(prompt: &str) -> String {
    strip_controls(prompt).into_owned()
}

pub fn build_codex_memory_command(model: &str, workdir: &Path) -> Invocation {
    let mut args: Vec<OsString> = [
        "exec",
        "--json",
        "--sandbox",
        "read-only",
        "--ephemeral",
        "--ignore-user-config",
        "--skip-git-repo-check",
        "-C",
    ]
    .map(OsString::from)
    .into();
    args.push(workdir.into());
    args.extend(
        [
            "--model",
            model,
            "--disable",
            "shell_tool",
            "--disable",
            "apps",
            "--disable",
            "multi_agent",
            "-c",
            "web_search=\"disabled\"",
            "-",
        ]
        .map(OsString::from),
    );
    Invocation {
        program: CODEX_COMMAND,
        args,
        env: Vec::new(),
    }
}

pub fn build_memory_command(
    model: &str,
    home: Option<&Path>,
    config_dir: Option<&Path>,
) -> Invocation {
    memory_command(model, None, &Isolation::for_memory(config_dir), home)
}

pub fn build_nap_command(
    model: &str,
    home: Option<&Path>,
    config_dir: Option<&Path>,
) -> Invocation {
    let system = ralph_core::chat_memory::nap_system();
    memory_command(model, Some(&system), &Isolation::for_nap(config_dir), home)
}

pub fn build_tag_command(
    model: &str,
    system_prompt: &str,
    home: Option<&Path>,
    config_dir: Option<&Path>,
) -> Invocation {
    let isolation = Isolation::for_memory(config_dir);
    memory_command(model, Some(system_prompt), &isolation, home)
}

fn memory_command(
    model: &str,
    system_prompt: Option<&str>,
    isolation: &Isolation,
    home: Option<&Path>,
) -> Invocation {
    let mut args: Vec<OsString> = ["--verbose", "--model", model].map(OsString::from).into();
    if let Some(system_prompt) = system_prompt {
        args.extend(["--system-prompt", system_prompt].map(OsString::from));
    }
    args.extend(isolation_args(home, false, isolation, false).map(OsString::from));
    args.extend(
        [
            "--tools",
            "",
            "--mcp-config",
            "{\"mcpServers\":{}}",
            "--strict-mcp-config",
            "--output-format",
            "stream-json",
            "-p",
        ]
        .map(OsString::from),
    );
    Invocation {
        program: CLAUDE_COMMAND,
        args,
        env: config_dir_env(isolation.config_dir.as_deref(), home)
            .into_iter()
            .collect(),
    }
}

pub fn build_chat_command(
    system_prompt_file: &Path,
    extra_args: &[String],
    permission_mode: ClaudeMode,
    home: Option<&Path>,
    isolation: &Isolation,
) -> Invocation {
    let mcptools = extra_args
        .windows(2)
        .any(|args| args[0] == "--permission-prompt-tool" && args[1] == "stdio");
    let mut args: Vec<OsString> = ["--verbose", "--permission-mode", permission_mode.as_str()]
        .map(OsString::from)
        .into();
    args.extend(isolation_args(home, true, isolation, mcptools).map(OsString::from));
    args.push("--append-system-prompt-file".into());
    args.push(system_prompt_file.into());
    args.extend(extra_args.iter().map(OsString::from));
    args.extend(
        [
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "-p",
        ]
        .map(OsString::from),
    );
    let mut env = vec![(CHAT_DISABLE_BACKGROUND_TASKS_ENV, "1".into())];
    env.extend(config_dir_env(isolation.config_dir.as_deref(), home));
    Invocation {
        program: CLAUDE_COMMAND,
        args,
        env,
    }
}

pub fn with_chat_inbox(mut invocation: Invocation, store: &Path) -> io::Result<Invocation> {
    let (key, inbox) = inbox_env(&std::fs::canonicalize(store)?);
    invocation.env.push((key, inbox.into()));
    Ok(invocation)
}

pub fn build_command(
    permission_mode: ClaudeMode,
    config_dir: Option<&Path>,
    home: Option<&Path>,
) -> Invocation {
    Invocation {
        program: CLAUDE_COMMAND,
        args: [
            "--verbose",
            "--permission-mode",
            permission_mode.as_str(),
            "--output-format",
            "stream-json",
            "-p",
        ]
        .map(OsString::from)
        .into(),
        env: config_dir_env(config_dir, home).into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(invocation: Invocation) -> String {
        invocation
            .env
            .iter()
            .map(|(key, value)| format!("{key}={}", value.to_string_lossy()))
            .chain([invocation.program.to_string()])
            .chain(
                invocation
                    .args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned()),
            )
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn chat_inbox_variable_names_the_canonical_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("chat's store");
        std::fs::create_dir(&store).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(&store, &alias).unwrap();
        let canonical = std::fs::canonicalize(&store).unwrap().join("inbox");
        let invocation =
            with_chat_inbox(build_command(ClaudeMode::Auto, None, None), &alias).unwrap();
        assert_eq!(invocation.env, [("RALPH_INBOX", canonical.into())]);
    }

    #[test]
    fn chat_inbox_command_rejects_a_missing_store() {
        let dir = tempfile::tempdir().unwrap();
        let ask = build_command(ClaudeMode::Auto, None, None);
        assert!(with_chat_inbox(ask, &dir.path().join("missing")).is_err());
    }

    #[test]
    fn codex_memory_command_is_isolated() {
        let cmd = line(build_codex_memory_command("model-x", Path::new("/tmp/a b")));
        assert_eq!(
            cmd,
            r#"codex exec --json --sandbox read-only --ephemeral --ignore-user-config --skip-git-repo-check -C /tmp/a b --model model-x --disable shell_tool --disable apps --disable multi_agent -c web_search="disabled" -"#
        );
    }

    #[test]
    fn a_hostile_curator_value_is_exactly_one_argument() {
        let hostile = "$(touch /tmp/pwned); id #";
        let home = Some(Path::new("/home/u"));
        for (invocation, program) in [
            (build_nap_command(hostile, home, None), "claude"),
            (build_memory_command(hostile, home, None), "claude"),
            (build_tag_command(hostile, hostile, home, None), "claude"),
            (
                build_codex_memory_command(hostile, Path::new(hostile)),
                "codex",
            ),
        ] {
            assert_eq!(invocation.program, program);
            let flag = invocation.args.iter().position(|arg| arg == "--model");
            assert_eq!(invocation.args[flag.unwrap() + 1], hostile);
        }
    }

    #[test]
    fn build_command_is_plain_ask() {
        let cmd = line(build_command(ClaudeMode::BypassPermissions, None, None));
        assert!(cmd.contains("--permission-mode bypassPermissions"));
        assert!(cmd.ends_with(" -p"), "{cmd}");
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--append-system-prompt"));
        assert!(!cmd.contains("--setting-sources"));
    }

    #[test]
    fn tagger_is_haiku_5_5_with_the_prompt_as_the_system_prompt_and_no_tools() {
        let cmd = line(build_tag_command(
            "claude-haiku-5-5",
            "Label 'x'.",
            Some(Path::new("/home/u")),
            None,
        ));
        assert!(cmd.contains("--model claude-haiku-5-5 "), "{cmd}");
        assert!(cmd.contains("--system-prompt Label 'x'. "), "{cmd}");
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains("--tools  --mcp-config"));
        assert!(cmd.contains("--mcp-config {\"mcpServers\":{}} --strict-mcp-config"));
        assert!(cmd.ends_with("--output-format stream-json -p"), "{cmd}");
        assert!(!cmd.contains("--resume"));
    }

    #[test]
    fn tagger_uses_the_model_it_is_given() {
        let cmd = line(build_tag_command("model-x", "p", None, None));
        assert!(cmd.contains("--model model-x "), "{cmd}");
        assert!(!cmd.contains("haiku"), "{cmd}");
    }

    #[test]
    fn nap_curator_gets_the_nap_rules_as_its_system_prompt() {
        let cmd = line(build_nap_command("haiku", Some(Path::new("/home/u")), None));
        let system = ralph_core::chat_memory::nap_system();
        assert!(cmd.contains(&format!("--system-prompt {system} ")), "{cmd}");
        assert!(cmd.contains("Invent nothing"), "{cmd}");
        assert!(cmd.contains("--model haiku "), "{cmd}");
        assert!(cmd.contains("--tools  --mcp-config"), "{cmd}");
        assert!(cmd.ends_with("--output-format stream-json -p"), "{cmd}");
    }

    #[test]
    fn nap_curator_drops_every_claude_md_and_auto_memory_but_merge_and_tag_keep_them() {
        let home = Some(Path::new("/home/u"));
        let nap = line(build_nap_command("haiku", home, None));
        for needle in [
            r#""autoMemoryEnabled":false"#,
            r#""**/CLAUDE.md""#,
            r#""**/CLAUDE.local.md""#,
        ] {
            assert!(nap.contains(needle), "{needle}: {nap}");
        }
        assert_eq!(nap.matches("--settings ").count(), 1, "{nap}");
        for other in [
            build_memory_command("haiku", home, None),
            build_tag_command("claude-haiku-5-5", "p", home, None),
        ]
        .map(line)
        {
            assert!(!other.contains("autoMemoryEnabled"), "{other}");
            assert!(!other.contains("**/CLAUDE.md"), "{other}");
        }
    }

    #[test]
    fn isolation_args_turns_auto_memory_off_only_when_asked() {
        let off = r#""autoMemoryEnabled":false"#;
        let nap = isolation_args(None, false, &Isolation::for_nap(None), false).join(" ");
        assert!(nap.contains(off), "{nap}");
        let chat = isolation_args(None, true, &Isolation::default(), false).join(" ");
        assert!(!chat.contains("autoMemoryEnabled"), "{chat}");
    }

    #[test]
    fn merge_curator_keeps_the_default_system_prompt() {
        let cmd = line(build_memory_command(
            "haiku",
            Some(Path::new("/home/u")),
            None,
        ));
        assert!(!cmd.contains("--system-prompt"), "{cmd}");
    }

    #[test]
    fn memory_curator_is_haiku_and_isolated_without_tools() {
        let cmd = line(build_memory_command(
            "haiku",
            Some(Path::new("/home/u")),
            None,
        ));
        assert!(cmd.contains("--model haiku"));
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains("claudeMdExcludes"));
        assert!(cmd.contains("/home/u/.claude/CLAUDE.md"));
        assert!(cmd.contains("--tools  --mcp-config"));
        assert!(cmd.contains("--strict-mcp-config"));
        assert!(cmd.ends_with("--output-format stream-json -p"), "{cmd}");
        assert!(!cmd.contains("--agent"));
        assert!(!cmd.contains("--resume"));
    }

    #[test]
    fn chat_command_is_fresh_and_isolated() {
        let mcp = vec!["--strict-mcp-config".to_string()];
        let cmd = line(build_chat_command(
            Path::new("/s/chat-1/system-prompt.md"),
            &mcp,
            ClaudeMode::Plan,
            Some(Path::new("/home/u")),
            &Isolation::default(),
        ));
        assert!(cmd.starts_with("CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 claude "));
        assert!(cmd.contains("--permission-mode plan "));
        assert!(!cmd.contains("--resume") && !cmd.contains("--agent"));
        assert!(cmd.contains("--setting-sources project,local"));
        assert!(cmd.contains(
            "--append-system-prompt-file /s/chat-1/system-prompt.md --strict-mcp-config"
        ));
        assert!(cmd.contains("--input-format stream-json --output-format stream-json"));
        assert!(cmd.ends_with(" -p"));
        assert!(cmd.contains(r#"--settings {"autoCompactEnabled":false,"#));
    }

    #[test]
    fn chat_mcptools_uses_exact_ask_rules_and_stdio() {
        let config =
            serde_json::json!({"mcpServers": {"mcptools": {}, "ralph-chat": {}, "linear": {}}});
        let args = ralph_core::chat::build_mcp_args(Path::new("/s/mcp.json"), &config);
        for (mode, user_settings) in [
            (ClaudeMode::Auto, false),
            (ClaudeMode::DontAsk, false),
            (ClaudeMode::Auto, true),
        ] {
            let isolation = Isolation {
                user_settings,
                ..Isolation::default()
            };
            let invocation =
                build_chat_command(Path::new("/s/p.md"), &args, mode, None, &isolation);
            let flag = invocation.args.iter().position(|arg| arg == "--settings");
            let settings = invocation.args[flag.unwrap() + 1].to_str().unwrap();
            let settings: serde_json::Value = serde_json::from_str(settings).unwrap();
            let cmd = line(invocation);
            assert!(cmd.contains("--permission-prompt-tool stdio"), "{cmd}");
            assert_eq!(cmd.matches("--settings ").count(), 1);
            assert_eq!(
                settings,
                serde_json::json!({
                    "autoCompactEnabled": false,
                    "permissions": {"ask": ["mcp__mcptools__execute", "mcp__mcptools__call_tool", "Bash(mcptools:*)"]},
                    "totalTokensReminder": "off"
                })
            );
            assert!(cmd.contains(&format!("--permission-mode {} ", mode.as_str())));
            assert!(cmd.contains(if user_settings {
                "--setting-sources user,project,local"
            } else {
                "--setting-sources project,local"
            }));
        }
        let allowed: std::collections::BTreeSet<_> = args[3].split(',').collect();
        assert_eq!(
            allowed,
            [
                "mcp__linear__*",
                "mcp__mcptools__find_tools",
                "mcp__ralph-chat__*"
            ]
            .into()
        );
        let mut without_discovery = args.clone();
        without_discovery[3] = "mcp__ralph-chat__*".into();
        let cmd = line(build_chat_command(
            Path::new("/s/p.md"),
            &without_discovery,
            ClaudeMode::Auto,
            None,
            &Isolation::default(),
        ));
        assert!(cmd.contains("mcp__mcptools__execute"));
        let no_mcptools = ralph_core::chat::build_mcp_args(
            Path::new("/s/mcp.json"),
            &serde_json::json!({"mcpServers": {"ralph-chat": {}}}),
        );
        let cmd = line(build_chat_command(
            Path::new("/s/p.md"),
            &no_mcptools,
            ClaudeMode::Auto,
            None,
            &Isolation::default(),
        ));
        assert!(!cmd.contains("--permission-prompt-tool"));
        assert!(!cmd.contains("mcp__mcptools__"));
    }

    #[test]
    fn chat_command_passes_agents_and_keeps_isolation() {
        let cmd = line(build_chat_command(
            Path::new("/s/p.md"),
            &ralph_core::chat::build_agents_args(),
            ClaudeMode::Auto,
            Some(Path::new("/home/u")),
            &Isolation::default(),
        ));
        assert!(cmd.contains("--agents"));
        assert!(cmd.contains("--setting-sources project,local"));
    }

    #[test]
    fn chat_and_nap_turn_off_the_total_tokens_reminder() {
        let chat = line(build_chat_command(
            Path::new("/s/p.md"),
            &[],
            ClaudeMode::Auto,
            None,
            &Isolation::default(),
        ));
        let nap = line(build_memory_command("haiku", None, None));
        for cmd in [chat, nap] {
            assert!(cmd.contains(r#""totalTokensReminder":"off""#), "{cmd}");
        }
    }

    const SECRET_LIKE_PROMPT: &str = "use KEY=abc123 and it's \"$x\"\nñ";

    #[test]
    fn prompts_stay_off_every_command_line() {
        let home = Some(Path::new("/home/u"));
        let commands = [
            line(build_command(ClaudeMode::Auto, None, home)),
            line(build_memory_command("haiku", home, None)),
            line(build_chat_command(
                Path::new("/s/p.md"),
                &[],
                ClaudeMode::Auto,
                home,
                &Isolation::default(),
            )),
        ];
        for cmd in commands {
            assert!(!cmd.contains("abc123"), "{cmd}");
            assert!(!cmd.contains("--append-system-prompt "), "{cmd}");
            assert!(cmd.ends_with(" -p"), "{cmd}");
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
            line(build_command(
                ClaudeMode::Auto,
                None,
                Some(Path::new("/home/u"))
            )),
            "claude --verbose --permission-mode auto --output-format stream-json -p"
        );
        assert_eq!(
            line(build_memory_command(
                "haiku",
                Some(Path::new("/home/u")),
                None
            )),
            r#"claude --verbose --model haiku --setting-sources project,local --settings {"claudeMdExcludes":["/home/u/.claude/CLAUDE.md"],"totalTokensReminder":"off"} --tools  --mcp-config {"mcpServers":{}} --strict-mcp-config --output-format stream-json -p"#
        );
        assert_eq!(
            line(build_chat_command(Path::new("/s/p.md"), &[], ClaudeMode::Auto, None, &Isolation::default())),
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 claude --verbose --permission-mode auto --setting-sources project,local --settings {\"autoCompactEnabled\":false,\"totalTokensReminder\":\"off\"} --append-system-prompt-file /s/p.md --input-format stream-json --output-format stream-json -p"
        );
    }

    #[test]
    fn config_dir_prefixes_every_command_and_moves_the_exclude() {
        let home = Some(Path::new("/home/u"));
        let dir = Some(Path::new("~/alt dir"));
        let ask = line(build_command(ClaudeMode::Auto, dir, home));
        assert!(ask.starts_with("CLAUDE_CONFIG_DIR=/home/u/alt dir claude --verbose "));
        let memory = line(build_memory_command("haiku", home, dir));
        assert!(memory.starts_with("CLAUDE_CONFIG_DIR=/home/u/alt dir claude "));
        assert!(memory.contains(r#"["/home/u/alt dir/CLAUDE.md"]"#));
        let isolation = Isolation::for_chat(&ChatSection::default(), Path::new("/work"), dir);
        let chat = line(build_chat_command(
            Path::new("/s/p.md"),
            &[],
            ClaudeMode::Auto,
            home,
            &isolation,
        ));
        assert!(chat.starts_with(
            "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 CLAUDE_CONFIG_DIR=/home/u/alt dir claude "
        ));
        assert!(chat.contains(r#""claudeMdExcludes":["/home/u/alt dir/CLAUDE.md"]"#));
        assert!(!chat.contains("/.claude/CLAUDE.md"));
    }

    fn chat_args(chat: ChatSection) -> String {
        isolation_args(
            Some(Path::new("/home/u")),
            true,
            &Isolation::for_chat(&chat, Path::new("/work"), None),
            false,
        )
        .join(" ")
    }

    #[test]
    fn project_claude_md_off_writes_the_shared_exclude_list() {
        let chat = ChatSection {
            project_claude_md: false,
            ..ChatSection::default()
        };
        let isolation = Isolation::for_chat(&chat, Path::new("/work"), None);
        assert_eq!(
            isolation.project_excludes,
            chat_project_excludes(Path::new("/work"))
        );
    }

    #[test]
    fn default_chat_isolation_is_unchanged() {
        assert_eq!(
            chat_args(ChatSection::default()),
            r#"--setting-sources project,local --settings {"autoCompactEnabled":false,"claudeMdExcludes":["/home/u/.claude/CLAUDE.md"],"totalTokensReminder":"off"}"#
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
            r#"--setting-sources project,local --settings {"autoCompactEnabled":false,"totalTokensReminder":"off"}"#
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
            r#"--setting-sources user,project,local --settings {"autoCompactEnabled":false,"claudeMdExcludes":["/work/CLAUDE.md","/work/CLAUDE.local.md","/work/.claude/CLAUDE.md"],"totalTokensReminder":"off"}"#
        );
    }
}
