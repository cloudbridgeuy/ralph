use crate::config::AppConfig;
use crate::subprocess::{spawn_detached, timed_output};
use crate::verified_agents;
use crate::worker_mcp::Inbox;
use crate::worker_verify::version_probe;
use ralph_core::worker::{
    check_agent_version, check_version, drift_action, drift_alert, drift_entry, known_fail_warning,
    last_verified_version, lock_action, opencode_probed_version, probe_text, store_inbox,
    tmux_claude_version, CheckResult, DriftAction, DriftEntry, DriftOutcome, HarnessId, LockAction,
    VersionGate, VersionProbe, CLAUDE_COMMAND, CLAUDE_TMUX_GATE, FREE_VERIFY_LIMIT,
    OPENCODE_COMMAND, OPENCODE_GATE, TIME_LIMIT_CHECK, VERSION_PROBE_LIMIT,
};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn probe(command: &str) -> VersionProbe {
    version_probe(timed_output(
        Command::new(command).arg("--version"),
        VERSION_PROBE_LIMIT,
    ))
}

pub(crate) fn probe_output(command: &str) -> Result<String, String> {
    probe_text(&format!("{command} --version"), &probe(command))
}

pub(crate) fn gate(
    gate: &VersionGate,
    found: &str,
    inbox: &Path,
) -> Result<Option<String>, String> {
    check_agent_version(gate, found, &verified_agents::load())
        .map(|warning| warning.map(|warning| on_drift(gate.agent, found, inbox, warning)))
}

pub(crate) fn soft_warning(
    gate: &VersionGate,
    version: Result<String, String>,
    inbox: &Path,
) -> Option<String> {
    version
        .ok()
        .and_then(|found| self::gate(gate, &found, inbox).unwrap_or_else(Some))
}

pub(crate) fn codex_gate(
    found: &str,
    warning_file: Option<&Path>,
) -> Result<Option<String>, String> {
    let warning = check_version(found, &verified_agents::load())?;
    Ok(match (warning, warning_file.and_then(store_inbox)) {
        (Some(warning), Some(inbox)) => Some(on_drift("codex", found, &inbox, warning)),
        (warning, _) => warning,
    })
}

pub(crate) fn spawn_warning(harness: HarnessId, inbox: &Path) -> Option<String> {
    match harness {
        HarnessId::Claude => soft_warning(
            &CLAUDE_TMUX_GATE,
            tmux_claude_version(&probe(CLAUDE_COMMAND)),
            inbox,
        ),
        HarnessId::Opencode => soft_warning(
            &OPENCODE_GATE,
            opencode_probed_version(&probe(OPENCODE_COMMAND)),
            inbox,
        ),
        HarnessId::Cursor | HarnessId::Codex | HarnessId::ClaudeHeadless => None,
    }
}

fn on_drift(agent: &str, found: &str, inbox: &Path, warning: String) -> String {
    match drift_action(
        agent,
        found,
        &verified_agents::load(),
        &verified_agents::load_drift(),
    ) {
        DriftAction::None => warning,
        DriftAction::KnownFail { checks } => known_fail_warning(agent, found, &checks),
        DriftAction::Verify => {
            if verify_on_drift() {
                start_verify(agent, found, inbox);
            }
            warning
        }
    }
}

fn verify_on_drift() -> bool {
    AppConfig::load().map_or(true, |config| config.workers.verify_on_drift)
}

fn create_lock(path: &Path) -> bool {
    path.parent()
        .is_some_and(|dir| fs::create_dir_all(dir).is_ok())
        && OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .is_ok()
}

fn acquire_lock(agent: &str) -> bool {
    let path = verified_agents::drift_lock_path(agent);
    let age = fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok());
    match lock_action(age) {
        LockAction::Busy => false,
        LockAction::Acquire => create_lock(&path),
        LockAction::Replace => fs::remove_file(&path).is_ok() && create_lock(&path),
    }
}

fn release_lock(agent: &str) {
    fs::remove_file(verified_agents::drift_lock_path(agent)).ok();
}

fn start_verify(agent: &str, found: &str, inbox: &Path) {
    if !acquire_lock(agent) {
        return;
    }
    let started = std::env::current_exe().and_then(|exe| {
        spawn_detached(
            Command::new(exe)
                .args(["worker", "verify", agent, "--drift", found, "--inbox"])
                .arg(inbox)
                .process_group(0),
        )
    });
    if started.is_err() {
        release_lock(agent);
    }
}

#[derive(Clone)]
pub(crate) struct DriftRun {
    version: String,
    inbox: PathBuf,
    done: Arc<AtomicBool>,
}

impl DriftRun {
    pub(crate) fn new(version: String, inbox: PathBuf) -> Self {
        Self {
            version,
            inbox,
            done: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn watch(&self, agent: &'static str) {
        let run = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(FREE_VERIFY_LIMIT);
            let results = [CheckResult::fail(TIME_LIMIT_CHECK, "verify ran too long")];
            if run.finish(agent, &results, false) {
                end_own_group();
            }
        });
    }

    pub(crate) fn conclude(&self, agent: &str, results: &[CheckResult], passed: bool) {
        if !self.finish(agent, results, passed) {
            std::thread::sleep(Duration::from_secs(10));
        }
    }

    fn finish(&self, agent: &str, results: &[CheckResult], passed: bool) -> bool {
        if self.done.swap(true, Ordering::SeqCst) {
            return false;
        }
        let entry = drift_entry(
            agent,
            &self.version,
            results,
            passed,
            &chrono::Utc::now().to_rfc3339(),
        );
        post_alert(agent, &entry, &self.inbox);
        verified_agents::save_drift(entry).ok();
        release_lock(agent);
        true
    }
}

fn post_alert(agent: &str, entry: &DriftEntry, inbox: &Path) {
    if let DriftOutcome::Failed { checks } = &entry.outcome {
        let old = last_verified_version(&verified_agents::load(), agent);
        let report = drift_alert(agent, old.as_deref(), &entry.version, checks, &entry.at);
        Inbox::new(inbox.to_path_buf()).post(&report, 0).ok();
    }
}

fn end_own_group() {
    let pid = std::process::id() as libc::pid_t;
    if unsafe { libc::getpgrp() } == pid {
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    std::process::exit(1)
}

pub(crate) fn record_manual(
    agent: &str,
    version: Option<&str>,
    results: &[CheckResult],
    passed: bool,
) {
    if let Some(version) = version {
        let entry = drift_entry(
            agent,
            version,
            results,
            passed,
            &chrono::Utc::now().to_rfc3339(),
        );
        verified_agents::save_drift(entry).ok();
    }
}
