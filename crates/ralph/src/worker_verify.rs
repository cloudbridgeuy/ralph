use crate::cli::VerifyAgent;
use crate::{acp_worker, codex_worker, verified_agents};
use ralph_core::worker::{render_check, render_summary, spec_passed, verified_entry, CheckResult};
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone, Default)]
pub(crate) struct Checks(Arc<Mutex<Vec<CheckResult>>>);

impl Checks {
    pub(crate) fn add(&self, result: CheckResult) {
        println!("{}", render_check(&result));
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(result);
    }

    pub(crate) fn results(&self) -> Vec<CheckResult> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

pub(crate) fn run(agent: VerifyAgent, live: bool) -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let name = agent.name();
    println!("verifying {name} in {}", dir.path().display());
    let checks = Checks::default();
    let version = match agent {
        VerifyAgent::Cursor => acp_worker::verify(live, dir.path(), &checks),
        VerifyAgent::Codex => codex_worker::verify(live, dir.path(), &checks),
    };
    let passed = spec_passed(&checks.results(), version.as_deref());
    let shown = version.as_deref().unwrap_or("unknown version");
    println!("{}", render_summary(name, shown, live, passed));
    if !passed {
        return Err(format!("{name} {shown} failed the verify spec").into());
    }
    if let Some(version) = version {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        verified_agents::save(verified_entry(name, &version, live, &today))?;
        println!("recorded {name} {version} in the verified agents file");
    }
    Ok(())
}
