use crate::config::AppConfig;
use crate::private_file::check_path;
use crate::subprocess::{kill_registered_groups, spawn_detached, timed_output};
use crate::verified_agents;
use crate::watches::locked;
use crate::worker_mcp::Inbox;
use crate::worker_verify::version_probe;
use ralph_core::worker::{
    check_agent_version, check_version, daily_check_due, daily_gate, daily_probe_plan,
    daily_verify_version, drift_action, drift_alert, drift_entry, drift_watchdog_limit,
    finish_plan, known_fail_warning, last_check_time, last_verified_version, lock_claim,
    opencode_probed_version, probe_text, probed_version, record_after_post, store_inbox,
    tmux_claude_version, CheckResult, DriftAction, DriftEntry, DriftOutcome, FinishPlan, HarnessId,
    LockClaim, LockError, VersionGate, VersionProbe, CLAUDE_COMMAND, CLAUDE_TMUX_GATE,
    CURSOR_AGENT, DAILY_CHECK_LIMIT, LOCK_CHECK, OPENCODE_COMMAND, OPENCODE_GATE, TIME_LIMIT_CHECK,
    VERSION_PROBE_LIMIT,
};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const WATCHDOG_WAIT: Duration = Duration::from_secs(10);

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

fn probed(harness: HarnessId) -> Result<String, String> {
    match harness {
        HarnessId::Claude => tmux_claude_version(&probe(CLAUDE_COMMAND)),
        HarnessId::Opencode => opencode_probed_version(&probe(OPENCODE_COMMAND)),
        HarnessId::Cursor => {
            probed_version(&CURSOR_AGENT, &probe(CURSOR_AGENT.command)).map(str::to_string)
        }
        HarnessId::Codex | HarnessId::ClaudeHeadless => Err("not probed".to_string()),
    }
}

pub(crate) fn run_daily(inbox: &Path) {
    verified_agents::save_last_check(chrono::Utc::now()).ok();
    std::thread::spawn(|| {
        std::thread::sleep(DAILY_CHECK_LIMIT);
        end_own_group();
    });
    for harness in daily_probe_plan() {
        let Some(gate) = daily_gate(harness) else {
            continue;
        };
        if let Some(found) = daily_verify_version(
            gate,
            probed(harness),
            &verified_agents::load(),
            &verified_agents::load_drift(),
        ) {
            start_verify(gate.agent, &found, inbox);
        }
    }
}

pub(crate) struct DailyTick {
    enabled: bool,
    last: Option<chrono::DateTime<chrono::Utc>>,
}

impl DailyTick {
    pub(crate) fn start() -> Self {
        Self {
            enabled: verify_on_drift(),
            last: last_check_time(&verified_agents::load_drift()),
        }
    }

    pub(crate) fn tick(&mut self, inbox: &Path) {
        let now = chrono::Utc::now();
        if daily_check_due(self.enabled, self.last, now) {
            self.last = Some(now);
            spawn_daily(inbox);
        }
    }
}

fn spawn_daily(inbox: &Path) {
    std::env::current_exe()
        .and_then(|exe| {
            spawn_detached(
                Command::new(exe)
                    .args(["worker", "drift-check", "--inbox"])
                    .arg(inbox)
                    .process_group(0),
            )
        })
        .ok();
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

pub(crate) fn verify_on_drift() -> bool {
    AppConfig::load().map_or(true, |config| config.workers.verify_on_drift)
}

fn acquire_lock(agent: &str) -> Result<File, LockError> {
    let path = verified_agents::drift_lock_path(agent);
    let io = |error: &dyn std::fmt::Display| LockError::Io(format!("{}: {error}", path.display()));
    let parent = path.parent().ok_or_else(|| io(&"no parent directory"))?;
    fs::create_dir_all(parent).map_err(|error| io(&error))?;
    check_path(&path).map_err(|error| io(&error))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|error| io(&error))?;
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => LockError::WouldBlock,
        TryLockError::Error(error) => io(&error),
    })?;
    Ok(file)
}

fn start_verify(agent: &str, found: &str, inbox: &Path) {
    std::env::current_exe()
        .and_then(|exe| {
            spawn_detached(
                Command::new(exe)
                    .args(["worker", "verify", agent, "--drift", found, "--inbox"])
                    .arg(inbox)
                    .process_group(0),
            )
        })
        .ok();
}

#[derive(Clone)]
pub(crate) struct DriftRun {
    version: String,
    inbox: PathBuf,
    _lock: Option<Arc<File>>,
    done: Arc<AtomicBool>,
    watchdog: Arc<Mutex<Option<Receiver<()>>>>,
}

impl DriftRun {
    pub(crate) fn new(version: String, inbox: PathBuf) -> Self {
        Self {
            version,
            inbox,
            _lock: None,
            done: Arc::new(AtomicBool::new(false)),
            watchdog: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn claim(self, agent: &str) -> Option<Self> {
        match acquire_lock(agent).map_err(|error| lock_claim(&error)) {
            Ok(lock) => Some(Self {
                _lock: Some(Arc::new(lock)),
                ..self
            }),
            Err(LockClaim::Busy) => None,
            Err(LockClaim::Broken(detail)) => {
                self.record(
                    agent,
                    &[CheckResult::fail(LOCK_CHECK, detail)],
                    false,
                    false,
                );
                None
            }
        }
    }

    pub(crate) fn watch(&self, agent: &'static str) {
        let run = self.clone();
        let (finished, ended) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let _finished = finished;
            std::thread::sleep(drift_watchdog_limit());
            let results = [CheckResult::fail(TIME_LIMIT_CHECK, "verify ran too long")];
            if run.plan(false).record_drift {
                run.record(agent, &results, false, false);
                end_own_group();
            }
        });
        *locked(&self.watchdog) = Some(ended);
    }

    pub(crate) fn plan(&self, passed: bool) -> FinishPlan {
        finish_plan(!self.done.swap(true, Ordering::SeqCst), passed)
    }

    pub(crate) fn await_watchdog(&self) {
        let ended = locked(&self.watchdog).take();
        if let Some(ended) = ended {
            ended.recv_timeout(WATCHDOG_WAIT).ok();
        }
    }

    pub(crate) fn record(
        &self,
        agent: &str,
        results: &[CheckResult],
        passed: bool,
        save_failed: bool,
    ) {
        let entry = drift_entry(
            agent,
            &self.version,
            results,
            passed,
            &chrono::Utc::now().to_rfc3339(),
        );
        let posted = post_alert(agent, &entry, &self.inbox);
        if record_after_post(&entry.outcome, posted, save_failed) {
            verified_agents::save_drift(entry).ok();
        }
    }
}

fn post_alert(agent: &str, entry: &DriftEntry, inbox: &Path) -> bool {
    let DriftOutcome::Failed { checks } = &entry.outcome else {
        return false;
    };
    let old = last_verified_version(&verified_agents::load(), agent);
    let report = drift_alert(agent, old.as_deref(), &entry.version, checks, &entry.at);
    Inbox::new(inbox.to_path_buf()).post(&report, 0).is_ok()
}

fn end_own_group() {
    kill_registered_groups();
    let pid = std::process::id() as libc::pid_t;
    // Safety: getpgrp has no arguments and cannot fail.
    if unsafe { libc::getpgrp() } == pid {
        // Safety: this process leads the group, so killpg reaches this process
        // and only the children that stayed in its group. Children in their
        // own process group are reached only by kill_registered_groups above.
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    std::process::exit(1)
}

pub(crate) fn record_manual(
    agent: &str,
    version: Option<&str>,
    results: &[CheckResult],
    passed: bool,
    save_failed: bool,
) {
    if let Some(version) = version.filter(|_| !save_failed) {
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
