use super::kind::HarnessId;
use super::ticket::Tracker;
use super::tools::ReportStatus;
use crate::workers_view::Diff;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use std::path::{Path, PathBuf};

const WORKERS_LOG_VERSION: u32 = 1;
const ANALYTICS_DIR: &str = "analytics";
const WORKERS_LOG_FILE: &str = "workers.jsonl";

pub fn workers_log_path(store: &Path) -> PathBuf {
    store.join(ANALYTICS_DIR).join(WORKERS_LOG_FILE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkPhase {
    Plan,
    Build,
    Fix,
    LiveCheck,
}

impl WorkPhase {
    pub const ALL: [WorkPhase; 4] = [
        WorkPhase::Plan,
        WorkPhase::Build,
        WorkPhase::Fix,
        WorkPhase::LiveCheck,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WorkPhase::Plan => "plan",
            WorkPhase::Build => "build",
            WorkPhase::Fix => "fix",
            WorkPhase::LiveCheck => "live-check",
        }
    }

    pub fn parse(text: &str) -> Option<WorkPhase> {
        WorkPhase::ALL
            .into_iter()
            .find(|phase| phase.as_str() == text)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum CloseOutcome {
    Landed,
    Abandoned,
    Forced,
}

fn close_outcome(abandon: bool, force: bool, foreign_owner: bool) -> CloseOutcome {
    if abandon {
        CloseOutcome::Abandoned
    } else if force && foreign_owner {
        CloseOutcome::Forced
    } else {
        CloseOutcome::Landed
    }
}

/// New branch -> tip; existing branch -> merge-base, never the tip; None on failure.
pub fn spawn_base_sha(
    created_branch: bool,
    tip: Option<&str>,
    merge_base: Option<&str>,
) -> Option<String> {
    let chosen = if created_branch { tip } else { merge_base };
    chosen.filter(|sha| !sha.is_empty()).map(str::to_string)
}

/// The git range of a worker's own commits, `<base_sha>..<branch>`; None unless base_sha is a valid hex id.
pub fn diff_range(base_sha: Option<&str>, branch: &str) -> Option<String> {
    base_sha
        .filter(|sha| {
            ((7..=40).contains(&sha.len()) || sha.len() == 64)
                && sha.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .map(|sha| format!("{sha}..{branch}"))
}

/// Turn-end snapshots after the spawn snapshot (turn 0), or `None` when the
/// worker has no turn snapshots.
pub fn turn_count(harness: HarnessId, turn_ids: &[u64]) -> Option<u64> {
    if harness == HarnessId::ClaudeHeadless {
        return None;
    }
    turn_ids.iter().max().copied()
}

fn duration_secs(spawn_at: Option<u64>, closed_at: u64) -> Option<u64> {
    spawn_at
        .filter(|spawned| *spawned > 0)
        .and_then(|spawned| closed_at.checked_sub(spawned))
}

#[derive(Debug, Clone, Copy)]
pub struct LogKey<'a> {
    pub store: &'a str,
    pub worker_id: &'a str,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub struct SpawnFacts<'a> {
    pub key: LogKey<'a>,
    pub ticket: Option<&'a str>,
    pub tracker: Option<Tracker>,
    pub harness: HarnessId,
    pub model: Option<&'a str>,
    pub base_sha: Option<&'a str>,
    pub scheduled: bool,
    pub phase: Option<WorkPhase>,
}

#[derive(Debug, Clone, Copy)]
pub struct CloseFacts<'a> {
    pub key: LogKey<'a>,
    pub abandon: bool,
    pub force: bool,
    pub foreign_owner: bool,
    pub commit_count: Option<u64>,
    pub dirty: Option<bool>,
    pub spawn_at: Option<u64>,
    pub diff: Option<&'a Diff>,
    pub turns: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct ReplyFacts<'a> {
    pub key: LogKey<'a>,
    pub harness: HarnessId,
    pub interrupt: bool,
    pub via_permission: bool,
    pub text_bytes: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ReportFacts<'a> {
    pub key: LogKey<'a>,
    pub status: ReportStatus,
    pub text_bytes: usize,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum LifecycleKind {
    Spawn {
        ticket: Option<String>,
        tracker: Option<Tracker>,
        harness: String,
        model: Option<String>,
        base_sha: Option<String>,
        scheduled: bool,
        phase: Option<WorkPhase>,
    },
    Close {
        outcome: CloseOutcome,
        commit_count: Option<u64>,
        dirty: Option<bool>,
        duration_s: Option<u64>,
        spawn_at: Option<String>,
        files: Option<u64>,
        insertions: Option<u64>,
        deletions: Option<u64>,
        turns: Option<u64>,
        merged_sha: Option<String>,
        ci: Option<String>,
    },
    Reply {
        harness: String,
        interrupt: bool,
        via_permission: bool,
        text_bytes: u64,
    },
    Report {
        status: String,
        text_bytes: u64,
        seq: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LifecycleEvent {
    pub v: u32,
    pub at: String,
    pub store: String,
    pub worker_id: String,
    #[serde(flatten)]
    kind: LifecycleKind,
}

fn event(key: &LogKey, kind: LifecycleKind) -> LifecycleEvent {
    LifecycleEvent {
        v: WORKERS_LOG_VERSION,
        at: key.at.to_rfc3339_opts(SecondsFormat::Secs, true),
        store: key.store.to_string(),
        worker_id: key.worker_id.to_string(),
        kind,
    }
}

pub fn spawn_event(facts: &SpawnFacts) -> LifecycleEvent {
    event(
        &facts.key,
        LifecycleKind::Spawn {
            ticket: facts.ticket.map(str::to_string),
            tracker: facts.tracker,
            harness: facts.harness.as_str().to_string(),
            model: facts.model.map(str::to_string),
            base_sha: facts.base_sha.map(str::to_string),
            scheduled: facts.scheduled,
            phase: facts.phase,
        },
    )
}

pub fn close_event(facts: &CloseFacts) -> LifecycleEvent {
    let closed_at = u64::try_from(facts.key.at.timestamp()).unwrap_or(0);
    event(
        &facts.key,
        LifecycleKind::Close {
            outcome: close_outcome(facts.abandon, facts.force, facts.foreign_owner),
            commit_count: facts.commit_count,
            dirty: facts.dirty,
            duration_s: duration_secs(facts.spawn_at, closed_at),
            spawn_at: facts
                .spawn_at
                .and_then(|secs| DateTime::from_timestamp(i64::try_from(secs).ok()?, 0))
                .map(|moment| moment.to_rfc3339_opts(SecondsFormat::Secs, true)),
            files: facts.diff.map(|diff| to_u64(diff.files)),
            insertions: facts.diff.map(|diff| to_u64(diff.added)),
            deletions: facts.diff.map(|diff| to_u64(diff.deleted)),
            turns: facts.turns,
            merged_sha: None,
            ci: None,
        },
    )
}

fn to_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

pub fn via_permission(request_given: bool, interrupt: bool, awaiting: bool) -> bool {
    !interrupt && (request_given || awaiting)
}

pub fn reply_event(facts: &ReplyFacts) -> LifecycleEvent {
    event(
        &facts.key,
        LifecycleKind::Reply {
            harness: facts.harness.as_str().to_string(),
            interrupt: facts.interrupt,
            via_permission: facts.via_permission,
            text_bytes: to_u64(facts.text_bytes),
        },
    )
}

pub fn report_event(facts: &ReportFacts) -> LifecycleEvent {
    event(
        &facts.key,
        LifecycleKind::Report {
            status: facts.status.as_str().to_string(),
            text_bytes: to_u64(facts.text_bytes),
            seq: facts.seq,
        },
    )
}

pub fn encode_event(event: &LifecycleEvent) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn key(secs: i64) -> LogKey<'static> {
        LogKey {
            store: "/s",
            worker_id: "GUZ-1",
            at: at(secs),
        }
    }

    fn spawn() -> SpawnFacts<'static> {
        SpawnFacts {
            key: key(1_700_000_000),
            ticket: Some("GUZ-1"),
            tracker: Some(Tracker::Linear),
            harness: HarnessId::ClaudeHeadless,
            model: Some("opus"),
            base_sha: Some("abc123"),
            scheduled: false,
            phase: Some(WorkPhase::LiveCheck),
        }
    }

    fn close() -> CloseFacts<'static> {
        CloseFacts {
            key: key(1_700_000_100),
            abandon: false,
            force: false,
            foreign_owner: false,
            commit_count: Some(3),
            dirty: Some(false),
            spawn_at: Some(1_700_000_000),
            turns: Some(5),
            diff: Some(&Diff {
                added: 12,
                deleted: 4,
                files: 2,
                untracked: 0,
            }),
        }
    }

    fn row(event: &LifecycleEvent) -> Value {
        let line = encode_event(event).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn the_log_sits_in_the_analytics_dir() {
        assert_eq!(
            workers_log_path(Path::new("/s")),
            PathBuf::from("/s/analytics/workers.jsonl")
        );
    }

    #[test]
    fn a_spawn_row_has_exactly_the_documented_fields() {
        assert_eq!(
            row(&spawn_event(&spawn())),
            json!({
                "v": 1,
                "at": "2023-11-14T22:13:20Z",
                "store": "/s",
                "worker_id": "GUZ-1",
                "event": "spawn",
                "ticket": "GUZ-1",
                "tracker": "linear",
                "harness": "claude-headless",
                "model": "opus",
                "base_sha": "abc123",
                "scheduled": false,
                "phase": "live-check"
            })
        );
    }

    #[test]
    fn phase_parses_only_the_four_names() {
        let parsed: Vec<_> = ["plan", "build", "fix", "live-check"]
            .map(WorkPhase::parse)
            .into();
        assert_eq!(parsed, WorkPhase::ALL.map(Some));
        for bad in ["live check", "Plan", "", "review"] {
            assert_eq!(WorkPhase::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn unknown_spawn_values_are_null() {
        let facts = SpawnFacts {
            ticket: None,
            tracker: None,
            model: None,
            base_sha: None,
            phase: None,
            ..spawn()
        };
        let value = row(&spawn_event(&facts));
        for field in ["ticket", "tracker", "model", "base_sha", "phase"] {
            assert_eq!(value[field], Value::Null, "{field}");
        }
    }

    #[test]
    fn a_close_row_has_exactly_the_documented_fields() {
        assert_eq!(
            row(&close_event(&close())),
            json!({
                "v": 1,
                "at": "2023-11-14T22:15:00Z",
                "store": "/s",
                "worker_id": "GUZ-1",
                "event": "close",
                "outcome": "landed",
                "commit_count": 3,
                "dirty": false,
                "duration_s": 100,
                "spawn_at": "2023-11-14T22:13:20Z",
                "files": 2,
                "insertions": 12,
                "deletions": 4,
                "turns": 5,
                "merged_sha": null,
                "ci": null
            })
        );
    }

    #[test]
    fn unknown_close_values_are_null_not_zero() {
        let facts = CloseFacts {
            commit_count: None,
            dirty: None,
            spawn_at: None,
            diff: None,
            turns: None,
            ..close()
        };
        let value = row(&close_event(&facts));
        for field in [
            "commit_count",
            "dirty",
            "duration_s",
            "spawn_at",
            "files",
            "insertions",
            "deletions",
            "turns",
        ] {
            assert_eq!(value.get(field), Some(&Value::Null), "{field}");
        }
    }

    #[test]
    fn the_diff_range_starts_at_the_recorded_base_sha() {
        assert_eq!(
            diff_range(Some("abc1234"), "worker/GUZ-1").as_deref(),
            Some("abc1234..worker/GUZ-1")
        );
        let full = "a".repeat(40);
        assert_eq!(
            diff_range(Some(&full), "worker/GUZ-1"),
            Some(format!("{full}..worker/GUZ-1"))
        );
    }

    #[test]
    fn the_diff_range_accepts_a_64_hex_sha256_id() {
        let full = "b".repeat(64);
        assert_eq!(
            diff_range(Some(&full), "worker/GUZ-1"),
            Some(format!("{full}..worker/GUZ-1"))
        );
    }

    #[test]
    fn the_diff_range_refuses_a_base_sha_that_is_not_7_to_40_or_64_hex() {
        for bad in [
            "--output=/tmp/x",
            "abc123",
            &"a".repeat(41),
            &"a".repeat(63),
            &"a".repeat(65),
            "xyz1234",
            "abc 1234",
        ] {
            assert_eq!(diff_range(Some(bad), "worker/GUZ-1"), None, "{bad}");
        }
    }

    #[test]
    fn a_fresh_branch_records_its_tip_as_the_base() {
        assert_eq!(
            spawn_base_sha(true, Some("tip1234"), Some("mb12345")).as_deref(),
            Some("tip1234")
        );
    }

    #[test]
    fn an_existing_branch_records_the_merge_base_not_the_tip() {
        assert_eq!(
            spawn_base_sha(false, Some("tip1234"), Some("mb12345")).as_deref(),
            Some("mb12345")
        );
    }

    #[test]
    fn an_existing_branch_without_a_merge_base_records_nothing() {
        assert_eq!(spawn_base_sha(false, Some("tip1234"), None), None);
        assert_eq!(spawn_base_sha(false, Some("tip1234"), Some("")), None);
    }

    #[test]
    fn the_diff_range_is_none_without_a_base_sha() {
        assert_eq!(diff_range(None, "worker/GUZ-1"), None);
        assert_eq!(diff_range(Some(""), "worker/GUZ-1"), None);
    }

    #[test]
    fn the_turn_count_is_the_highest_snapshot_id() {
        assert_eq!(turn_count(HarnessId::Claude, &[0]), Some(0));
        assert_eq!(turn_count(HarnessId::Claude, &[0, 1, 2, 3]), Some(3));
        assert_eq!(turn_count(HarnessId::Codex, &[12, 13, 14]), Some(14));
    }

    #[test]
    fn the_turn_count_is_null_without_snapshots_or_for_claude_headless() {
        assert_eq!(turn_count(HarnessId::Claude, &[]), None);
        assert_eq!(turn_count(HarnessId::ClaudeHeadless, &[0]), None);
        assert_eq!(turn_count(HarnessId::ClaudeHeadless, &[0, 1]), None);
    }

    #[test]
    fn the_outcome_follows_abandon_then_force_on_a_foreign_worker() {
        assert_eq!(close_outcome(false, false, false), CloseOutcome::Landed);
        assert_eq!(close_outcome(true, false, false), CloseOutcome::Abandoned);
        assert_eq!(close_outcome(true, true, true), CloseOutcome::Abandoned);
        assert_eq!(close_outcome(false, true, true), CloseOutcome::Forced);
        assert_eq!(close_outcome(false, true, false), CloseOutcome::Landed);
        assert_eq!(close_outcome(false, false, true), CloseOutcome::Landed);
    }

    #[test]
    fn the_close_row_carries_the_derived_outcome() {
        let abandoned = CloseFacts {
            abandon: true,
            ..close()
        };
        assert_eq!(row(&close_event(&abandoned))["outcome"], "abandoned");
        let forced = CloseFacts {
            force: true,
            foreign_owner: true,
            ..close()
        };
        assert_eq!(row(&close_event(&forced))["outcome"], "forced");
    }

    #[test]
    fn the_duration_is_the_close_time_minus_the_spawn_time() {
        assert_eq!(duration_secs(Some(10), 75), Some(65));
        assert_eq!(duration_secs(Some(75), 75), Some(0));
        assert_eq!(duration_secs(Some(80), 75), None);
        assert_eq!(duration_secs(Some(0), 75), None);
        assert_eq!(duration_secs(None, 75), None);
    }

    #[test]
    fn a_row_carries_no_cost_or_session_fields() {
        let spawned = row(&spawn_event(&spawn()));
        let closed = row(&close_event(&close()));
        for value in [spawned, closed] {
            for field in ["cost", "tokens", "session_id", "text", "reply_count"] {
                assert!(value.get(field).is_none(), "{field}");
            }
        }
    }

    fn reply() -> ReplyFacts<'static> {
        ReplyFacts {
            key: key(1_700_000_050),
            harness: HarnessId::Codex,
            interrupt: false,
            via_permission: false,
            text_bytes: 5,
        }
    }

    #[test]
    fn a_reply_row_has_the_length_of_the_text_and_not_the_text() {
        let line = encode_event(&reply_event(&reply())).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            json!({
                "v": 1,
                "at": "2023-11-14T22:14:10Z",
                "store": "/s",
                "worker_id": "GUZ-1",
                "event": "reply",
                "harness": "codex",
                "interrupt": false,
                "via_permission": false,
                "text_bytes": 5
            })
        );
    }

    #[test]
    fn a_reply_counts_bytes_not_chars() {
        let facts = ReplyFacts {
            text_bytes: "h\u{e9}llo".len(),
            interrupt: true,
            via_permission: true,
            ..reply()
        };
        let value = row(&reply_event(&facts));
        assert_eq!(value["text_bytes"], 6);
        assert_eq!(value["interrupt"], true);
        assert_eq!(value["via_permission"], true);
    }

    #[test]
    fn a_reply_is_a_permission_answer_when_asked_or_awaited() {
        assert!(via_permission(true, false, false));
        assert!(via_permission(false, false, true));
        assert!(via_permission(true, false, true));
        assert!(!via_permission(false, false, false));
        assert!(!via_permission(true, true, false));
        assert!(!via_permission(false, true, true));
    }

    #[test]
    fn a_report_row_has_the_status_length_and_sequence() {
        let facts = ReportFacts {
            key: key(1_700_000_060),
            status: ReportStatus::Blocked,
            text_bytes: 12,
            seq: 3,
        };
        let line = encode_event(&report_event(&facts)).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            json!({
                "v": 1,
                "at": "2023-11-14T22:14:20Z",
                "store": "/s",
                "worker_id": "GUZ-1",
                "event": "report",
                "status": "blocked",
                "text_bytes": 12,
                "seq": 3
            })
        );
    }

    #[test]
    fn every_report_status_is_named() {
        for (status, name) in [
            (ReportStatus::Done, "done"),
            (ReportStatus::Blocked, "blocked"),
            (ReportStatus::Question, "question"),
        ] {
            let facts = ReportFacts {
                key: key(0),
                status,
                text_bytes: 0,
                seq: 1,
            };
            assert_eq!(row(&report_event(&facts))["status"], name);
        }
    }
}
