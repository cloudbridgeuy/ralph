use crate::subprocess::timed_output;
use crate::verified_agents;
use crate::worker_verify::version_probe;
use ralph_core::worker::{
    check_agent_version, opencode_probed_version, probe_text, soft_version_warning,
    tmux_claude_version, HarnessId, VersionGate, VersionProbe, CLAUDE_COMMAND, CLAUDE_TMUX_GATE,
    OPENCODE_COMMAND, OPENCODE_GATE, VERSION_PROBE_LIMIT,
};
use std::process::Command;

pub(crate) fn probe(command: &str) -> VersionProbe {
    version_probe(timed_output(
        Command::new(command).arg("--version"),
        VERSION_PROBE_LIMIT,
    ))
}

pub(crate) fn probe_output(command: &str) -> Result<String, String> {
    probe_text(&format!("{command} --version"), &probe(command))
}

pub(crate) fn gate(gate: &VersionGate, found: &str) -> Result<Option<String>, String> {
    check_agent_version(gate, found, &verified_agents::load())
}

pub(crate) fn soft_warning(gate: &VersionGate, version: Result<String, String>) -> Option<String> {
    version
        .ok()
        .and_then(|found| soft_version_warning(gate, &found, &verified_agents::load()))
}

pub(crate) fn spawn_warning(harness: HarnessId) -> Option<String> {
    match harness {
        HarnessId::Claude => soft_warning(
            &CLAUDE_TMUX_GATE,
            tmux_claude_version(&probe(CLAUDE_COMMAND)),
        ),
        HarnessId::Opencode => soft_warning(
            &OPENCODE_GATE,
            opencode_probed_version(&probe(OPENCODE_COMMAND)),
        ),
        HarnessId::Cursor | HarnessId::Codex | HarnessId::ClaudeHeadless => None,
    }
}
