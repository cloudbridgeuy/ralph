use super::{
    claude_worker_settings, decode_claude_version, task_dir_path, worker_launch, ActivityMark,
    ActivityState, CheckResult, ClaudeShell, ClaudeWorkerArgs, UsageSink, VersionProbe,
    WorkerMarkers, ATTACHMENTS_DIR, CLAUDE_PERMISSION_PROMPT, VERSION_PROBE_LIMIT,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CLAUDE_REQUIRED_CHECKS: &[&str] = &["version", "tmux", "flags", "session start"];
pub const CLAUDE_VERIFY_FLAGS: [&str; 5] = [
    "--permission-mode",
    "--mcp-config",
    "--settings",
    "--add-dir",
    "--append-system-prompt",
];
pub const CLAUDE_VERIFY_MCP: &str = r#"{"mcpServers":{}}"#;
const VERIFY_ID: &str = "verify";
const VERIFY_PERMISSION_MODE: &str = "default";
pub const CLAUDE_VERIFY_PERMISSION_PROMPT: &str = CLAUDE_PERMISSION_PROMPT;

pub struct VerifyPaths {
    pub store: PathBuf,
    pub work: PathBuf,
    pub attachments: PathBuf,
    pub task_dir: PathBuf,
    pub settings: PathBuf,
    pub mcp_config: PathBuf,
    pub usage: PathBuf,
    pub markers: WorkerMarkers,
}

impl VerifyPaths {
    pub fn new(dir: &Path) -> Self {
        let store = dir.join("store");
        Self {
            work: dir.join("work"),
            attachments: store.join(ATTACHMENTS_DIR),
            task_dir: task_dir_path(&store, VERIFY_ID),
            settings: dir.join("settings.json"),
            mcp_config: dir.join("mcp.json"),
            usage: dir.join("usage.jsonl"),
            markers: WorkerMarkers::new(&store, VERIFY_ID),
            store,
        }
    }

    pub fn settings_json(&self, exe: &Path) -> Value {
        claude_worker_settings(
            exe,
            &self.markers,
            None,
            &UsageSink {
                store: &self.store,
                worker: VERIFY_ID,
                usage: &self.usage,
            },
        )
    }

    pub fn command(
        &self,
        shell: ClaudeShell,
        config_dir: Option<&Path>,
        model: Option<&str>,
    ) -> Vec<String> {
        let mut command = worker_launch(&ClaudeWorkerArgs {
            store: &self.store,
            mcp_config: &self.mcp_config,
            settings: &self.settings,
            permission_mode: VERIFY_PERMISSION_MODE,
            attachments: &self.attachments,
            task_dir: &self.task_dir,
            model,
            scheduled: false,
            shell,
            config_dir,
        });
        let at = command
            .iter()
            .position(|word| word == "--append-system-prompt")
            .unwrap_or(command.len());
        command.splice(
            at..at,
            ["--setting-sources", "project,local"].map(str::to_string),
        );
        command
    }
}

pub fn probe_text(command: &str, probe: &VersionProbe) -> Result<String, String> {
    match probe {
        VersionProbe::Exited(text) => Ok(text.clone()),
        VersionProbe::ExitedWithError(_) => Err(format!("`{command}` exited with an error status")),
        VersionProbe::Unreadable(_) => Err(format!("`{command}` printed output that is not UTF-8")),
        VersionProbe::TimedOut => Err(format!(
            "`{command}` did not answer within {}s",
            VERSION_PROBE_LIMIT.as_secs()
        )),
        VersionProbe::Failed(error) => Err(format!("could not run `{command}`: {error}")),
    }
}

pub fn tmux_claude_version(probe: &VersionProbe) -> Result<String, String> {
    let text = probe_text("claude --version", probe)?;
    decode_claude_version(&text)
        .map(str::to_string)
        .ok_or_else(|| "could not read the version from `claude --version`".to_string())
}

pub fn missing_flags(help: &str) -> Vec<&'static str> {
    CLAUDE_VERIFY_FLAGS
        .into_iter()
        .filter(|flag| !help.contains(flag))
        .collect()
}

pub fn flags_check(help: &str) -> CheckResult {
    match missing_flags(help).as_slice() {
        [] => CheckResult::pass(
            "flags",
            format!("`claude --help` lists {}", CLAUDE_VERIFY_FLAGS.join(", ")),
        ),
        missing => CheckResult::fail(
            "flags",
            format!("`claude --help` does not list {}", missing.join(", ")),
        ),
    }
}

pub fn startup_check(outcome: Result<bool, String>) -> CheckResult {
    match outcome {
        Ok(true) => CheckResult::pass(
            "session start",
            "folder trust answered if asked; the SessionStart hook wrote the ready marker, so claude accepted the settings",
        ),
        Ok(false) => CheckResult::fail(
            "session start",
            "the SessionStart hook wrote no ready marker before the startup timeout",
        ),
        Err(message) => CheckResult::fail("session start", message),
    }
}

pub fn stop_seen(activity: Option<&str>) -> bool {
    activity
        .and_then(ActivityMark::parse)
        .is_some_and(|mark| mark.state == ActivityState::Idle)
}

pub fn claude_prompt_check(stop_seen: bool, limit: Duration) -> CheckResult {
    if stop_seen {
        CheckResult::pass(
            "prompt/reply",
            "the Stop hook marked the turn idle after one prompt",
        )
    } else {
        CheckResult::fail(
            "prompt/reply",
            format!(
                "the Stop hook did not mark the turn idle within {}s",
                limit.as_secs()
            ),
        )
    }
}

pub fn claude_permission_check(
    blocked_seen: bool,
    stop_seen: bool,
    limit: Duration,
) -> CheckResult {
    let setup = "permission-mode default";
    match (blocked_seen, stop_seen) {
        (false, _) => CheckResult::fail(
            "permission",
            format!(
                "the PermissionRequest hook wrote no blocked marker within {}s although claude runs {setup} and allows no shell tool",
                limit.as_secs()
            ),
        ),
        (true, false) => CheckResult::fail(
            "permission",
            format!(
                "the request was allowed but the Stop hook did not mark the turn idle within {}s (setup: {setup})",
                limit.as_secs()
            ),
        ),
        (true, true) => CheckResult::pass(
            "permission",
            format!("the PermissionRequest hook marked the turn blocked; allowed; the Stop hook marked it idle (setup: {setup})"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = "Options:\n  --add-dir <dirs>\n  --append-system-prompt <prompt>\n  --mcp-config <configs>\n  --permission-mode <mode>\n  --settings <file-or-json>\n  --model <model>\n";

    #[test]
    fn help_text_lists_every_flag_the_worker_passes() {
        assert!(missing_flags(HELP).is_empty());
        assert_eq!(flags_check(HELP).verdict, super::super::Verdict::Pass);
    }

    #[test]
    fn help_text_missing_a_flag_names_it() {
        let help = HELP.replace("--settings", "--setting");
        assert_eq!(missing_flags(&help), vec!["--settings"]);
        let check = flags_check(&help);
        assert_eq!(check.verdict, super::super::Verdict::Fail);
        assert!(check.detail.contains("--settings"), "{}", check.detail);
    }

    #[test]
    fn verify_settings_hooks_write_only_to_the_verify_dir() {
        let dir = Path::new("/tmp/verify-dir");
        let exe = Path::new("/opt/ralph");
        let settings = VerifyPaths::new(dir).settings_json(exe).to_string();
        let paths: Vec<&str> = settings
            .split('\'')
            .skip(1)
            .step_by(2)
            .filter(|quoted| quoted.starts_with('/') && *quoted != "/opt/ralph")
            .collect();
        assert!(!paths.is_empty());
        for path in paths {
            assert!(path.starts_with("/tmp/verify-dir/"), "{path}");
        }
    }

    #[test]
    fn verify_launch_sends_no_task_prompt() {
        let paths = VerifyPaths::new(Path::new("/tmp/verify-dir"));
        let command = paths.command(ClaudeShell::default(), None, None);
        assert!(
            command
                .iter()
                .all(|word| !word.starts_with("Read your task")),
            "{command:?}"
        );
        assert_eq!(
            command.last().map(String::as_str),
            Some(super::super::WORKER_PROMPT)
        );
        assert!(command.iter().any(|word| word == "--append-system-prompt"));
    }

    #[test]
    fn verify_launch_passes_the_configured_model() {
        let paths = VerifyPaths::new(Path::new("/tmp/verify-dir"));
        let with = paths.command(ClaudeShell::default(), None, Some("claude-haiku-5-5"));
        let at = with.iter().position(|word| word == "--model").unwrap();
        assert_eq!(with[at + 1], "claude-haiku-5-5");
        let without = paths.command(ClaudeShell::default(), None, None);
        assert!(without.iter().all(|word| word != "--model"));
    }

    #[test]
    fn verify_launch_isolates_user_settings() {
        let paths = VerifyPaths::new(Path::new("/tmp/verify-dir"));
        let command = paths.command(ClaudeShell::default(), None, None);
        let at = command
            .iter()
            .position(|word| word == "--setting-sources")
            .unwrap();
        assert_eq!(command[at + 1], "project,local");
    }

    #[test]
    fn verify_permission_prompt_writes_a_file() {
        assert_eq!(
            CLAUDE_VERIFY_PERMISSION_PROMPT,
            "Run the shell command `touch verify-permission.txt` and reply with ok."
        );
    }

    #[test]
    fn worker_launch_does_not_restrict_setting_sources() {
        let paths = VerifyPaths::new(Path::new("/tmp/verify-dir"));
        let args = ClaudeWorkerArgs {
            store: &paths.store,
            mcp_config: &paths.mcp_config,
            settings: &paths.settings,
            permission_mode: "default",
            attachments: &paths.attachments,
            task_dir: &paths.task_dir,
            model: None,
            scheduled: false,
            shell: ClaudeShell::default(),
            config_dir: None,
        };
        for command in [worker_launch(&args), super::super::worker_command(&args)] {
            assert!(command.iter().all(|word| word != "--setting-sources"));
        }
    }
}
