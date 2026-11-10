use crate::worker::{verify_hint, VerifiedAgents, CLAUDE_GATE, CURSOR_AGENT};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const DRIFT_FILE: &str = "drift.json";
pub const DRIFT_CHECK_INTERVAL_HOURS: i64 = 24;

fn parse_version(version: &str) -> Option<(u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    parts
        .next()?
        .split('-')
        .next()?
        .parse::<u64>()
        .ok()
        .map(|_| (major, minor))
}

pub struct VersionGate {
    pub agent: &'static str,
    pub package: &'static str,
    pub min: (u64, u64),
    pub probed: &'static str,
    pub protocol: &'static str,
    pub verifiable: bool,
}

pub const CURSOR_GATE: VersionGate = VersionGate {
    agent: "cursor",
    package: "Cursor agent",
    min: (2026, 10),
    probed: CURSOR_AGENT.tested_version,
    protocol: "ACP",
    verifiable: true,
};

pub const OPENCODE_GATE: VersionGate = VersionGate {
    agent: "opencode",
    package: "opencode",
    min: (0, 0),
    probed: "2.2.5",
    protocol: "HTTP",
    verifiable: true,
};

pub const CLAUDE_TMUX_GATE: VersionGate = VersionGate {
    agent: "claude-tmux",
    package: "Claude Code",
    min: (0, 0),
    probed: CLAUDE_GATE.probed,
    protocol: "tmux",
    verifiable: true,
};

pub fn check_agent_version(
    gate: &VersionGate,
    found: &str,
    verified: &VerifiedAgents,
) -> Result<Option<String>, String> {
    let VersionGate {
        agent,
        package,
        min,
        probed,
        protocol,
        verifiable,
    } = gate;
    let parsed = parse_version(found)
        .ok_or_else(|| format!("could not read the {agent} version from `{found}`"))?;
    let probed_pair = parse_version(probed).unwrap_or(*min);
    match (parsed.cmp(min), parsed.cmp(&probed_pair)) {
        (std::cmp::Ordering::Less, _) => Err(format!(
            "{agent} {found} is too old; needs {package} >= {}.{} (probed {probed})",
            min.0, min.1
        )),
        (_, std::cmp::Ordering::Greater) if *verifiable && verified.is_verified(agent, found) => {
            Ok(None)
        }
        (_, std::cmp::Ordering::Greater) if *verifiable => Ok(Some(format!(
            "{agent} {found} is newer than the probed {probed}; the {protocol} protocol may differ; {}",
            verify_hint(agent)
        ))),
        (_, std::cmp::Ordering::Greater) => Ok(Some(format!(
            "{agent} {found} is newer than the probed {probed}; the {protocol} protocol may differ."
        ))),
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftOutcome {
    Passed,
    Failed { checks: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftEntry {
    pub agent: String,
    pub version: String,
    pub outcome: DriftOutcome,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftRecord {
    #[serde(default)]
    pub agents: Vec<DriftEntry>,
    #[serde(default)]
    pub last_check: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftAction {
    None,
    Verify,
    KnownFail { checks: Vec<String> },
}

pub fn drift_action(
    agent: &str,
    installed: &str,
    verified: &VerifiedAgents,
    drift: &DriftRecord,
) -> DriftAction {
    if verified.is_verified(agent, installed) {
        return DriftAction::None;
    }
    match drift
        .agents
        .iter()
        .find(|entry| entry.agent == agent && entry.version == installed)
    {
        Some(DriftEntry {
            outcome: DriftOutcome::Failed { checks },
            ..
        }) => DriftAction::KnownFail {
            checks: checks.clone(),
        },
        Some(DriftEntry {
            outcome: DriftOutcome::Passed,
            ..
        }) => DriftAction::None,
        None => DriftAction::Verify,
    }
}

pub fn record_outcome(drift: &DriftRecord, entry: DriftEntry) -> DriftRecord {
    let mut agents: Vec<DriftEntry> = drift
        .agents
        .iter()
        .filter(|known| !(known.agent == entry.agent && known.version == entry.version))
        .cloned()
        .collect();
    agents.push(entry);
    DriftRecord {
        agents,
        last_check: drift.last_check.clone(),
    }
}

pub fn drift_check_due(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    last.is_none_or(|last| now - last >= Duration::hours(DRIFT_CHECK_INTERVAL_HOURS))
}

#[cfg(test)]
mod tests {

    fn soft_version_warning(
        gate: &VersionGate,
        found: &str,
        verified: &VerifiedAgents,
    ) -> Option<String> {
        check_agent_version(gate, found, verified).unwrap_or_else(Some)
    }
    use super::*;
    use crate::worker::verified_entry;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn entry(agent: &str, version: &str, outcome: DriftOutcome) -> DriftEntry {
        DriftEntry {
            agent: agent.to_string(),
            version: version.to_string(),
            outcome,
            at: "2026-11-03T00:00:00Z".to_string(),
        }
    }

    fn failed(checks: &[&str]) -> DriftOutcome {
        DriftOutcome::Failed {
            checks: checks.iter().map(|check| check.to_string()).collect(),
        }
    }

    #[test]
    fn a_verified_version_needs_no_action() {
        let verified =
            VerifiedAgents::default().record(verified_entry("codex", "0.170.0", false, "d"));
        assert_eq!(
            drift_action("codex", "0.170.0", &verified, &DriftRecord::default()),
            DriftAction::None
        );
    }

    #[test]
    fn an_unverified_version_with_no_entry_needs_a_verify() {
        assert_eq!(
            drift_action(
                "codex",
                "0.170.0",
                &VerifiedAgents::default(),
                &DriftRecord::default()
            ),
            DriftAction::Verify
        );
    }

    #[test]
    fn a_recorded_failure_is_not_retried() {
        let drift = record_outcome(
            &DriftRecord::default(),
            entry("codex", "0.170.0", failed(&["initialize"])),
        );
        assert_eq!(
            drift_action("codex", "0.170.0", &VerifiedAgents::default(), &drift),
            DriftAction::KnownFail {
                checks: vec!["initialize".to_string()]
            }
        );
        assert_eq!(
            drift_action("codex", "0.171.0", &VerifiedAgents::default(), &drift),
            DriftAction::Verify
        );
        assert_eq!(
            drift_action("cursor", "0.170.0", &VerifiedAgents::default(), &drift),
            DriftAction::Verify
        );
    }

    #[test]
    fn a_recorded_pass_that_is_not_verified_starts_no_second_verify() {
        let drift = record_outcome(
            &DriftRecord::default(),
            entry("codex", "0.170.0", DriftOutcome::Passed),
        );
        assert_eq!(
            drift_action("codex", "0.170.0", &VerifiedAgents::default(), &drift),
            DriftAction::None
        );
        assert_eq!(
            drift_action("codex", "0.171.0", &VerifiedAgents::default(), &drift),
            DriftAction::Verify
        );
    }

    #[test]
    fn recording_replaces_the_same_agent_and_version() {
        let first = record_outcome(
            &DriftRecord::default(),
            entry("codex", "0.170.0", failed(&["a"])),
        );
        let other = record_outcome(&first, entry("codex", "0.171.0", DriftOutcome::Passed));
        let replaced = record_outcome(&other, entry("codex", "0.170.0", DriftOutcome::Passed));
        assert_eq!(replaced.agents.len(), 2);
        assert!(replaced
            .agents
            .contains(&entry("codex", "0.170.0", DriftOutcome::Passed)));
        assert!(replaced
            .agents
            .contains(&entry("codex", "0.171.0", DriftOutcome::Passed)));
    }

    #[test]
    fn recording_keeps_the_last_check() {
        let drift = DriftRecord {
            agents: Vec::new(),
            last_check: Some("2026-11-03T00:00:00Z".to_string()),
        };
        let next = record_outcome(&drift, entry("codex", "1.0.0", DriftOutcome::Passed));
        assert_eq!(next.last_check, drift.last_check);
    }

    #[test]
    fn the_daily_check_is_due_after_24_hours() {
        let now = at("2026-11-03T12:00:00Z");
        assert!(drift_check_due(None, now));
        assert!(!drift_check_due(Some(at("2026-11-03T00:00:00Z")), now));
        assert!(!drift_check_due(Some(at("2026-11-02T12:00:01Z")), now));
        assert!(drift_check_due(Some(at("2026-11-02T12:00:00Z")), now));
        assert!(drift_check_due(Some(at("2026-10-01T00:00:00Z")), now));
    }

    #[test]
    fn a_record_reads_back_and_tolerates_missing_fields() {
        assert_eq!(
            serde_json::from_str::<DriftRecord>("{}").unwrap(),
            DriftRecord::default()
        );
        let drift = record_outcome(
            &DriftRecord::default(),
            entry("codex", "0.170.0", failed(&["a", "b"])),
        );
        let text = serde_json::to_string(&drift).unwrap();
        assert_eq!(serde_json::from_str::<DriftRecord>(&text).unwrap(), drift);
    }

    #[test]
    fn the_gate_orders_versions() {
        let gate = VersionGate {
            agent: "x",
            package: "X",
            min: (1, 2),
            probed: "1.4.0",
            protocol: "p",
            verifiable: true,
        };
        let none = VerifiedAgents::default();
        assert_eq!(check_agent_version(&gate, "1.3.9", &none), Ok(None));
        assert!(check_agent_version(&gate, "1.1.0", &none).is_err());
        assert!(check_agent_version(&gate, "1.5.0", &none)
            .unwrap()
            .is_some());
        assert!(check_agent_version(&gate, "1.5", &none).is_err());
    }

    fn soft_gate() -> VersionGate {
        VersionGate {
            agent: "x",
            package: "X",
            min: (1, 2),
            probed: "1.4.0",
            protocol: "p",
            verifiable: true,
        }
    }

    #[test]
    fn a_soft_gate_never_fails_it_only_warns() {
        let none = VerifiedAgents::default();
        let gate = soft_gate();
        assert_eq!(soft_version_warning(&gate, "1.3.9", &none), None);
        let old = soft_version_warning(&gate, "1.1.0", &none).unwrap();
        assert!(old.contains("too old"), "{old}");
        let newer = soft_version_warning(&gate, "1.5.0", &none).unwrap();
        assert!(newer.contains("newer than the probed 1.4.0"), "{newer}");
        assert!(newer.ends_with("run `ralph worker verify x` to check this version."));
        let unreadable = soft_version_warning(&gate, "garbage", &none).unwrap();
        assert!(unreadable.contains("could not read"), "{unreadable}");
    }

    #[test]
    fn a_verify_pass_silences_the_soft_warning_for_that_version_only() {
        let verified =
            VerifiedAgents::default().record(verified_entry("x", "1.5.0", false, "2026-11-03"));
        let gate = soft_gate();
        assert_eq!(soft_version_warning(&gate, "1.5.0", &verified), None);
        assert!(soft_version_warning(&gate, "1.5.1", &verified).is_some());
    }

    #[test]
    fn the_cursor_gate_warns_on_a_new_version_until_verified() {
        let none = VerifiedAgents::default();
        assert_eq!(
            soft_version_warning(&CURSOR_GATE, CURSOR_AGENT.tested_version, &none),
            None
        );
        let warning = soft_version_warning(&CURSOR_GATE, "2026.11.01", &none).unwrap();
        assert!(warning.contains("2026.11.01"), "{warning}");
        let verified = VerifiedAgents::default().record(verified_entry(
            "cursor",
            "2026.11.01",
            false,
            "2026-11-03",
        ));
        assert_eq!(
            soft_version_warning(&CURSOR_GATE, "2026.11.01", &verified),
            None
        );
    }

    #[test]
    fn the_opencode_and_tmux_claude_gates_warn_on_unverified_newer_versions() {
        let none = VerifiedAgents::default();
        for (gate, probed, newer) in [
            (&OPENCODE_GATE, "2.2.5", "2.3.0"),
            (&CLAUDE_TMUX_GATE, "2.1.314", "2.2.0"),
        ] {
            assert_eq!(soft_version_warning(gate, probed, &none), None);
            let warning = soft_version_warning(gate, newer, &none).unwrap();
            assert!(warning.contains(newer), "{warning}");
            assert!(warning.contains(gate.agent), "{warning}");
            let verified =
                VerifiedAgents::default().record(verified_entry(gate.agent, newer, false, "d"));
            assert_eq!(soft_version_warning(gate, newer, &verified), None);
        }
        assert_eq!(soft_version_warning(&OPENCODE_GATE, "1.0.0", &none), None);
        assert_eq!(
            soft_version_warning(&CLAUDE_TMUX_GATE, "1.0.0", &none),
            None
        );
    }
}
