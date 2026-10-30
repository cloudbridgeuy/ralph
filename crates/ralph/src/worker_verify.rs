use crate::cli::VerifyAgent;
use crate::subprocess::kill_process_group;
use crate::watches::locked;
use crate::{acp_worker, codex_worker, verified_agents};
use ralph_core::worker::{
    finished_probe, missing_checks, render_check, render_summary, spec_passed, verified_entry,
    CheckResult, VerifiedAgent, VersionProbe, CODEX_REQUIRED_CHECKS, CURSOR_REQUIRED_CHECKS,
};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

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

pub(crate) fn timed_output(command: &mut Command, limit: Duration) -> VersionProbe {
    let deadline = Instant::now() + limit;
    let spawned = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return VersionProbe::Failed(error.to_string()),
    };
    let Some(mut stdout) = child.stdout.take() else {
        kill_process_group(&mut child);
        return VersionProbe::Failed("no stdout".to_string());
    };
    let (sender, receiver) = mpsc::channel();
    let reader = thread::Builder::new().spawn(move || {
        let mut buffer = Vec::new();
        let _ = sender.send(stdout.read_to_end(&mut buffer).map(|_| buffer));
    });
    if let Err(error) = reader {
        kill_process_group(&mut child);
        return VersionProbe::Failed(error.to_string());
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                kill_process_group(&mut child);
                return VersionProbe::TimedOut;
            }
            Err(error) => {
                kill_process_group(&mut child);
                return VersionProbe::Failed(error.to_string());
            }
        }
    };
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(stdout)) => finished_probe(status.success(), stdout),
        Ok(Err(error)) => VersionProbe::Failed(error.to_string()),
        Err(_) => VersionProbe::TimedOut,
    }
}

struct Outcome<'a> {
    name: &'a str,
    version: Option<&'a str>,
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
        live,
        passed,
    } = *outcome;
    let shown = version.unwrap_or("unknown version");
    let recorded = match version.filter(|_| passed) {
        Some(version) => record(verified_entry(name, version, live, today))
            .map(|()| format!("recorded {name} {version} in the verified agents file"))
            .map_err(|error| format!("{name} {version} passed but was not recorded: {error}")),
        None => Err(format!("{name} {shown} failed the verify spec")),
    };
    let summary = render_summary(name, shown, live, recorded.is_ok());
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
    let (version, required) = match agent {
        VerifyAgent::Cursor => (
            acp_worker::verify(live, dir.path(), &checks),
            CURSOR_REQUIRED_CHECKS,
        ),
        VerifyAgent::Codex => (
            codex_worker::verify(live, dir.path(), &checks),
            CODEX_REQUIRED_CHECKS,
        ),
    };
    let missing = missing_checks(&checks.results(), required);
    if !missing.is_empty() {
        checks.add(CheckResult::fail(
            "required",
            format!("these checks did not run: {}", missing.join(", ")),
        ));
    }
    let outcome = Outcome {
        name,
        version: version.as_deref(),
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

    fn passed(version: Option<&str>) -> Outcome<'_> {
        Outcome {
            name: "cursor",
            version,
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
}
