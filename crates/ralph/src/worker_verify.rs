use crate::cli::VerifyAgent;
use crate::config::AppConfig;
use crate::subprocess::TimedOutput;
use crate::watches::locked;
use crate::{
    acp_worker, claude_headless_worker, codex_worker, opencode_worker, verified_agents, workers,
};
use ralph_core::worker::{
    finished_probe, render_check, render_summary, required_failure, resolve_claude_model,
    resolve_opencode_model, spec_passed, verified_entry, CheckResult, Verdict, VerifiedAgent,
    VersionProbe, CLAUDE_HEADLESS_REQUIRED_CHECKS, CLAUDE_REQUIRED_CHECKS, CODEX_REQUIRED_CHECKS,
    CURSOR_REQUIRED_CHECKS, DEFAULT_OPENCODE_MODEL, OPENCODE_REQUIRED_CHECKS,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct Checks(Arc<Mutex<Vec<CheckResult>>>);

impl Checks {
    pub(crate) fn add(&self, result: CheckResult) {
        println!("{}", render_check(&result));
        locked(&self.0).push(result);
    }

    pub(crate) fn results(&self) -> Vec<CheckResult> {
        locked(&self.0).clone()
    }
}

pub(crate) fn step<T>(
    checks: &Checks,
    name: &'static str,
    outcome: Result<T, String>,
    detail: impl FnOnce(&T) -> String,
) -> Option<T> {
    match outcome {
        Ok(value) => {
            checks.add(CheckResult::pass(name, detail(&value)));
            Some(value)
        }
        Err(message) => {
            checks.add(CheckResult::fail(name, message));
            None
        }
    }
}

pub(crate) fn version_probe(output: TimedOutput) -> VersionProbe {
    match output {
        TimedOutput::Finished { success, stdout } => finished_probe(success, stdout),
        TimedOutput::TimedOut => VersionProbe::TimedOut,
        TimedOutput::Failed(error) => VersionProbe::Failed(error),
    }
}

fn resolved_model(
    agent: VerifyAgent,
    config: &Result<AppConfig, String>,
) -> (Option<String>, Option<String>) {
    match agent {
        VerifyAgent::Cursor | VerifyAgent::Codex => (None, None),
        VerifyAgent::Claude | VerifyAgent::ClaudeHeadless => resolve_claude_model(
            None,
            config
                .as_ref()
                .map(|config| config.claude.model.as_deref())
                .map_err(String::as_str),
        ),
        VerifyAgent::Opencode => {
            let (model, warning) = resolve_opencode_model(
                None,
                config
                    .as_ref()
                    .map(|config| config.opencode.model.as_deref())
                    .map_err(String::as_str),
            );
            (Some(model), warning)
        }
    }
}

fn model_label(agent: VerifyAgent, model: Option<&str>) -> Option<String> {
    match agent {
        VerifyAgent::Cursor | VerifyAgent::Codex => None,
        VerifyAgent::Claude | VerifyAgent::ClaudeHeadless => {
            Some(model.unwrap_or("claude's default model").to_string())
        }
        VerifyAgent::Opencode => model.map(str::to_string),
    }
}

struct Outcome<'a> {
    name: &'a str,
    version: Option<&'a str>,
    model: Option<&'a str>,
    live: bool,
    passed: bool,
}

fn conclude(
    outcome: &Outcome,
    today: &str,
    record: impl FnOnce(VerifiedAgent) -> Result<(), String>,
) -> (Vec<String>, Result<(), String>) {
    let Outcome {
        name,
        version,
        model,
        live,
        passed,
    } = *outcome;
    let shown = version.unwrap_or("unknown version");
    let described = match model {
        Some(model) => format!("{shown} with model {model}"),
        None => shown.to_string(),
    };
    let recorded = match version.filter(|_| passed) {
        Some(version) => record(verified_entry(name, version, live, today))
            .map(|()| format!("recorded {name} {version} in the verified agents file"))
            .map_err(|error| format!("{name} {version} passed but was not recorded: {error}")),
        None => Err(format!("{name} {shown} failed the verify spec")),
    };
    let summary = render_summary(name, &described, live, recorded.is_ok());
    match recorded {
        Ok(line) => (vec![summary, line], Ok(())),
        Err(error) => (vec![summary], Err(error)),
    }
}

pub(crate) fn run(agent: VerifyAgent, live: bool) -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let name = agent.name();
    println!("verifying {name} in {}", dir.path().display());
    let checks = Checks::default();
    let config = AppConfig::load().map_err(|error| error.to_string());
    let (model, warning) = resolved_model(agent, &config);
    if let Some(warning) = warning {
        checks.add(CheckResult::advisory("model", Verdict::Unknown, warning));
    }
    let (version, required) = match agent {
        VerifyAgent::Cursor => (
            acp_worker::verify(live, dir.path(), &checks),
            CURSOR_REQUIRED_CHECKS,
        ),
        VerifyAgent::Codex => (
            codex_worker::verify(live, dir.path(), &checks),
            CODEX_REQUIRED_CHECKS,
        ),
        VerifyAgent::ClaudeHeadless => (
            claude_headless_worker::verify(live, dir.path(), &checks, &config, model.as_deref()),
            CLAUDE_HEADLESS_REQUIRED_CHECKS,
        ),
        VerifyAgent::Opencode => (
            opencode_worker::verify(
                live,
                dir.path(),
                &checks,
                model.as_deref().unwrap_or(DEFAULT_OPENCODE_MODEL),
            ),
            OPENCODE_REQUIRED_CHECKS,
        ),
        VerifyAgent::Claude => (
            workers::verify(live, dir.path(), &checks, &config, model.as_deref()),
            CLAUDE_REQUIRED_CHECKS,
        ),
    };
    if let Some(failure) = required_failure(&checks.results(), required) {
        checks.add(failure);
    }
    let label = model_label(agent, model.as_deref());
    let outcome = Outcome {
        name,
        version: version.as_deref(),
        model: label.as_deref(),
        live,
        passed: spec_passed(&checks.results(), version.as_deref(), required),
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let (lines, result) = conclude(&outcome, &today, verified_agents::save);
    for line in lines {
        println!("{line}");
    }
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn passed(version: Option<&str>) -> Outcome<'_> {
        Outcome {
            name: "cursor",
            version,
            model: None,
            live: false,
            passed: version.is_some(),
        }
    }

    #[test]
    fn a_pass_that_cannot_be_recorded_never_prints_pass() {
        let (lines, result) = conclude(&passed(Some("1.0")), "2026-10-30", |_| {
            Err("refusing to overwrite verified-agents.json".to_string())
        });
        assert!(
            lines.iter().all(|line| !line.starts_with("PASS")),
            "{lines:?}"
        );
        assert!(result.unwrap_err().contains("refusing to overwrite"));
    }

    #[test]
    fn a_recorded_pass_prints_pass_and_a_failure_records_nothing() {
        let (lines, result) = conclude(&passed(Some("1.0")), "2026-10-30", |_| Ok(()));
        assert!(lines[0].starts_with("PASS: cursor 1.0"));
        assert_eq!(lines[1], "recorded cursor 1.0 in the verified agents file");
        assert!(result.is_ok());
        let (lines, result) = conclude(&passed(None), "2026-10-30", |_| {
            panic!("a failed spec must not be recorded")
        });
        assert!(lines[0].starts_with("FAIL: cursor unknown version"));
        assert!(result.is_err());
    }

    fn config_with(claude: Option<&str>, opencode: Option<&str>) -> Result<AppConfig, String> {
        let mut config = AppConfig::default();
        config.claude.model = claude.map(str::to_string);
        config.opencode.model = opencode.map(str::to_string);
        Ok(config)
    }

    #[test]
    fn claude_kinds_resolve_the_configured_model_like_spawn() {
        for agent in [VerifyAgent::Claude, VerifyAgent::ClaudeHeadless] {
            let config = config_with(Some("claude-haiku-5-5"), None);
            assert_eq!(
                resolved_model(agent, &config),
                (Some("claude-haiku-5-5".to_string()), None)
            );
            assert_eq!(
                resolved_model(agent, &config_with(None, None)),
                (None, None)
            );
        }
    }

    #[test]
    fn opencode_resolves_the_configured_model_or_the_default() {
        assert_eq!(
            resolved_model(
                VerifyAgent::Opencode,
                &config_with(None, Some("anthropic/claude-haiku-5-5"))
            ),
            (Some("anthropic/claude-haiku-5-5".to_string()), None)
        );
        assert_eq!(
            resolved_model(VerifyAgent::Opencode, &config_with(None, None)),
            (Some(DEFAULT_OPENCODE_MODEL.to_string()), None)
        );
    }

    #[test]
    fn a_bad_configured_model_warns_and_falls_back() {
        let (model, warning) =
            resolved_model(VerifyAgent::Claude, &config_with(Some("two words"), None));
        assert_eq!(model, None);
        assert!(warning.is_some());
    }

    #[test]
    fn the_summary_names_the_model_for_claude_and_opencode_only() {
        let named = |model| Outcome {
            model,
            ..passed(Some("2.1.316"))
        };
        let line = |outcome: Outcome| conclude(&outcome, "2026-10-30", |_| Ok(())).0.remove(0);
        assert!(line(named(Some("claude-haiku-5-5")))
            .starts_with("PASS: cursor 2.1.316 with model claude-haiku-5-5 ("));
        assert!(line(named(None)).starts_with("PASS: cursor 2.1.316 ("));
        assert_eq!(model_label(VerifyAgent::Cursor, None), None);
        assert_eq!(model_label(VerifyAgent::Codex, Some("x")), None);
        assert_eq!(
            model_label(VerifyAgent::Claude, None).as_deref(),
            Some("claude's default model")
        );
        assert_eq!(
            model_label(VerifyAgent::ClaudeHeadless, Some("m")).as_deref(),
            Some("m")
        );
    }

    #[test]
    fn the_cli_accepts_claude_as_a_verify_agent() {
        for agent in ["cursor", "codex", "claude-headless", "opencode", "claude"] {
            assert!(Cli::try_parse_from(["ralph", "worker", "verify", agent]).is_ok());
        }
        assert!(Cli::try_parse_from(["ralph", "worker", "verify", "tmux"]).is_err());
    }
}
