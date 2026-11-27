use crate::invoke::{build_chat_command, with_chat_inbox, Isolation};
use crate::private_file::write_private;
use crate::subprocess::{
    Invocation, OutputGate, Session, SpinnerSubprocessConfig, StreamingSubprocessResult,
    SubprocessError,
};
use ralph_core::cancel::CancelToken;
use ralph_core::chat::{build_agents_args, build_attachments_args};
use ralph_core::chat_backend::{Caps, CLAUDE};
use ralph_core::sanitize::strip_controls;
use ralph_core::stream::blocks::VerboseToolsConfig;
use ralph_core::turn_event::TurnSink;
use ralph_core::worker::ClaudeMode;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub trait ChatSession: Send {
    fn turn(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
    ) -> Result<StreamingSubprocessResult, SubprocessError>;
    fn ping(
        &mut self,
        prompt: &str,
        timeout: Duration,
    ) -> Result<StreamingSubprocessResult, SubprocessError>;
    fn is_alive(&mut self) -> bool;
    fn discard(&mut self);
}

impl ChatSession for Session {
    fn turn(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        Session::turn(self, prompt, cancel)
    }

    fn ping(
        &mut self,
        prompt: &str,
        timeout: Duration,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        Session::ping(self, prompt, timeout)
    }

    fn is_alive(&mut self) -> bool {
        Session::is_alive(self)
    }

    fn discard(&mut self) {
        Session::discard(self)
    }
}

pub struct ChatLaunch<'a> {
    pub store_dir: &'a Path,
    pub system: &'a str,
}

pub struct TurnIo {
    pub sink: Arc<dyn TurnSink>,
    pub gate: Option<Arc<OutputGate>>,
    pub show_tools: bool,
    pub terminated: fn() -> bool,
}

pub trait ChatBackend: Send + Sync {
    fn capabilities(&self) -> Caps;
    fn spawn(
        &self,
        launch: &ChatLaunch,
        io: &TurnIo,
    ) -> Result<Box<dyn ChatSession>, SubprocessError>;
}

pub struct ClaudeSlot {
    pub system_prompt_file: PathBuf,
    pub permission_mode: ClaudeMode,
    pub mcp_args: Vec<String>,
    pub plugin_args: Vec<String>,
    pub model_args: Vec<String>,
    pub isolation: Isolation,
}

fn claude_chat_command(slot: &ClaudeSlot, store_dir: &Path, home: Option<&Path>) -> Invocation {
    let args = [
        slot.mcp_args.clone(),
        build_attachments_args(store_dir),
        build_agents_args(),
        slot.plugin_args.clone(),
        slot.model_args.clone(),
    ]
    .concat();
    build_chat_command(
        &slot.system_prompt_file,
        &args,
        slot.permission_mode,
        home,
        &slot.isolation,
    )
}

fn write_system_prompt(path: &Path, system: &str) -> io::Result<PathBuf> {
    write_private(path.to_path_buf(), strip_controls(system).as_bytes())
}

impl ChatBackend for ClaudeSlot {
    fn capabilities(&self) -> Caps {
        CLAUDE
    }

    fn spawn(
        &self,
        launch: &ChatLaunch,
        io: &TurnIo,
    ) -> Result<Box<dyn ChatSession>, SubprocessError> {
        write_system_prompt(&self.system_prompt_file, launch.system)?;
        let command = with_chat_inbox(
            claude_chat_command(self, launch.store_dir, dirs::home_dir().as_deref()),
            launch.store_dir,
        )?;
        let config = SpinnerSubprocessConfig {
            command,
            timeout_secs: None,
            verbose_tools: VerboseToolsConfig::new(),
            show_tools: io.show_tools,
            terminated: io.terminated,
            sink: Arc::clone(&io.sink),
            gate: io.gate.clone(),
            interrupt: None,
            stdin: None,
        };
        Ok(Box::new(Session::spawn(&config)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invoke::Isolation;
    use std::os::unix::fs::PermissionsExt;

    const GOLDEN: &str = r#"CLAUDE_CODE_DISABLE_BACKGROUND_TASKS=1 CLAUDE_CONFIG_DIR=/c claude --verbose --permission-mode plan --setting-sources project,local --settings {"autoCompactEnabled":false,"autoMemoryEnabled":false,"claudeMdExcludes":["/c/CLAUDE.md","/p/x"],"totalTokensReminder":"off"} --append-system-prompt-file /s/system.md --mcp-config /s/mcp.json --add-dir /s/store/attachments --agents {"evidence-collector":{"description":"Collects bounded, read-only evidence: multi-file reads, broad searches, and test runs.","prompt":"Collect the requested evidence with read-only inspection. Report relevant files, commands, outputs, and uncertainty. Do not modify files or dispatch other agents.","model":"haiku"},"implementer":{"description":"Implements scoped code changes and verifies the result.","prompt":"Implement the requested scoped change. Read the affected code before editing. Change only the files the dispatch prompt names; preserve unrelated work. Run focused verification and report the evidence and remaining limits.","model":"sonnet"},"reviser":{"description":"Fixes or finishes work that fell short of the plan, then verifies and reports.","prompt":"Repair work that fell short of the plan. Compare the result with the plan, fix missing or broken parts yourself, change only what the plan requires or the defect touches, then verify and report what you changed and the evidence.","model":"opus"}} --plugin-dir /s/plug --model opus --input-format stream-json --output-format stream-json -p"#;

    #[test]
    fn claude_slot_command_matches_golden() {
        let slot = ClaudeSlot {
            system_prompt_file: PathBuf::from("/s/system.md"),
            permission_mode: ClaudeMode::Plan,
            mcp_args: vec!["--mcp-config".to_string(), "/s/mcp.json".to_string()],
            plugin_args: vec!["--plugin-dir".to_string(), "/s/plug".to_string()],
            model_args: vec!["--model".to_string(), "opus".to_string()],
            isolation: Isolation {
                user_settings: false,
                user_claude_md: false,
                project_excludes: vec![PathBuf::from("/p/x")],
                config_dir: Some(PathBuf::from("/c")),
                no_auto_memory: true,
            },
        };
        let invocation =
            claude_chat_command(&slot, Path::new("/s/store"), Some(Path::new("/home/u")));
        let line = invocation
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
            .join(" ");
        assert_eq!(line, GOLDEN);
    }

    #[test]
    fn system_prompt_file_holds_the_old_argument_bytes_privately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("system-prompt.md");
        let system = "rules\0 it's \"$HOME\"\n  ñ trailing \n";
        write_system_prompt(&path, &system.repeat(3)).unwrap();
        write_system_prompt(&path, system).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, b"rules it's \"$HOME\"\n  \xc3\xb1 trailing \n");
        assert_eq!(bytes, strip_controls(system).as_bytes());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
