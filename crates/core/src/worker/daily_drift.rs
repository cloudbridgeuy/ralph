use crate::worker::{
    check_agent_version, drift_action, drift_check_due, DriftAction, DriftRecord, HarnessId,
    VerifiedAgents, VersionGate, CLAUDE_TMUX_GATE, CURSOR_GATE, OPENCODE_GATE,
};
use chrono::{DateTime, Utc};
use std::time::Duration;

pub const DAILY_CHECK_LIMIT: Duration = Duration::from_secs(45);

pub fn daily_gate(harness: HarnessId) -> Option<&'static VersionGate> {
    match harness {
        HarnessId::ClaudeTmux => Some(&CLAUDE_TMUX_GATE),
        HarnessId::Opencode => Some(&OPENCODE_GATE),
        HarnessId::Cursor => Some(&CURSOR_GATE),
        HarnessId::Codex | HarnessId::Claude => None,
    }
}

pub fn daily_probe_plan() -> Vec<HarnessId> {
    HarnessId::ALL
        .into_iter()
        .filter(|harness| daily_gate(*harness).is_some())
        .collect()
}

pub fn daily_check_due(enabled: bool, last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    enabled && drift_check_due(last, now)
}

pub fn last_check_time(record: &DriftRecord) -> Option<DateTime<Utc>> {
    record
        .last_check
        .as_deref()
        .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
        .map(|stamp| stamp.with_timezone(&Utc))
}

pub fn stamp_check(record: &DriftRecord, now: DateTime<Utc>) -> DriftRecord {
    DriftRecord {
        agents: record.agents.clone(),
        last_check: Some(now.to_rfc3339()),
    }
}

pub fn daily_verify_version(
    gate: &VersionGate,
    probed: Result<String, String>,
    verified: &VerifiedAgents,
    drift: &DriftRecord,
) -> Option<String> {
    let found = probed.ok()?;
    let drifted = matches!(check_agent_version(gate, &found, verified), Ok(Some(_)));
    (drifted && drift_action(gate.agent, &found, verified, drift) == DriftAction::Verify)
        .then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{verified_entry, DriftEntry, DriftOutcome};

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn the_plan_probes_every_harness_with_a_version_decoder_and_skips_codex() {
        assert_eq!(
            daily_probe_plan(),
            vec![
                HarnessId::ClaudeTmux,
                HarnessId::Opencode,
                HarnessId::Cursor
            ]
        );
        assert!(daily_gate(HarnessId::Codex).is_none());
        assert!(daily_gate(HarnessId::Claude).is_none());
    }

    #[test]
    fn each_probed_harness_uses_its_own_gate() {
        assert_eq!(
            daily_gate(HarnessId::ClaudeTmux).map(|g| g.agent),
            Some("claude-tmux")
        );
        assert_eq!(
            daily_gate(HarnessId::Opencode).map(|g| g.agent),
            Some("opencode")
        );
        assert_eq!(
            daily_gate(HarnessId::Cursor).map(|g| g.agent),
            Some("cursor")
        );
    }

    #[test]
    fn the_check_is_due_only_when_enabled_and_old_or_missing() {
        let now = at("2026-11-03T12:00:00Z");
        assert!(daily_check_due(true, None, now));
        assert!(daily_check_due(true, Some(at("2026-11-02T12:00:00Z")), now));
        assert!(!daily_check_due(
            true,
            Some(at("2026-11-02T12:00:01Z")),
            now
        ));
        assert!(!daily_check_due(false, None, now));
        assert!(!daily_check_due(
            false,
            Some(at("2026-10-01T00:00:00Z")),
            now
        ));
    }

    #[test]
    fn the_last_check_reads_back_and_a_bad_stamp_counts_as_missing() {
        let now = at("2026-11-03T12:00:00Z");
        let stamped = stamp_check(&DriftRecord::default(), now);
        assert_eq!(last_check_time(&stamped), Some(now));
        let bad = DriftRecord {
            agents: Vec::new(),
            last_check: Some("yesterday".to_string()),
        };
        assert_eq!(last_check_time(&bad), None);
        assert_eq!(last_check_time(&DriftRecord::default()), None);
    }

    #[test]
    fn stamping_keeps_the_recorded_outcomes() {
        let entry = DriftEntry {
            agent: "claude-tmux".to_string(),
            version: "99.0.0".to_string(),
            outcome: DriftOutcome::Passed,
            at: "2026-11-03T00:00:00Z".to_string(),
        };
        let record = DriftRecord {
            agents: vec![entry.clone()],
            last_check: None,
        };
        let stamped = stamp_check(&record, at("2026-11-03T12:00:00Z"));
        assert_eq!(stamped.agents, vec![entry]);
    }

    #[test]
    fn a_drifted_unverified_version_starts_a_verify() {
        assert_eq!(
            daily_verify_version(
                &CLAUDE_TMUX_GATE,
                Ok("99.0.0".to_string()),
                &VerifiedAgents::default(),
                &DriftRecord::default()
            ),
            Some("99.0.0".to_string())
        );
    }

    #[test]
    fn a_verified_or_probed_or_known_failing_version_starts_nothing() {
        let verified =
            VerifiedAgents::default().record(verified_entry("claude-tmux", "99.0.0", false, "d"));
        let probed = CLAUDE_TMUX_GATE.probed.to_string();
        let failed = DriftRecord {
            agents: vec![DriftEntry {
                agent: "claude-tmux".to_string(),
                version: "98.0.0".to_string(),
                outcome: DriftOutcome::Failed {
                    checks: vec!["spawn".to_string()],
                },
                at: "2026-11-03T00:00:00Z".to_string(),
            }],
            last_check: None,
        };
        let none = DriftRecord::default();
        let run = |version: &str, verified, drift| {
            daily_verify_version(&CLAUDE_TMUX_GATE, Ok(version.to_string()), verified, drift)
        };
        assert_eq!(run("99.0.0", &verified, &none), None);
        let unverified = VerifiedAgents::default();
        assert_eq!(run(&probed, &unverified, &none), None);
        assert_eq!(run("98.0.0", &unverified, &failed), None);
    }

    #[test]
    fn a_missing_or_unreadable_probe_starts_nothing() {
        let none = DriftRecord::default();
        let verified = VerifiedAgents::default();
        for probed in [
            Err("could not run `claude --version`: not found".to_string()),
            Ok("garbage".to_string()),
        ] {
            assert_eq!(
                daily_verify_version(&CLAUDE_TMUX_GATE, probed, &verified, &none),
                None
            );
        }
    }
}
