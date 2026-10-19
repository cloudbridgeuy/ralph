use super::{
    opencode_blocked_content, permission_decision, PermissionAsked, PermissionDecision,
    OPENCODE_SESSION_DIR,
};
use crate::worker::{close_handoff, match_option, no_option_message, Handoff};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const OPENCODE_SPAWN_DIR: &str = "opencode-spawn";
pub const OPENCODE_PREFLIGHT_DIR: &str = "opencode-preflight";
pub const OPENCODE_HEALTHY_DIR: &str = "opencode-healthy";
pub const OPENCODE_CLOSE_DIR: &str = "opencode-close";
pub const OPENCODE_ERROR_DIR: &str = "opencode-error";
pub const OPENCODE_OWNER_DIR: &str = "opencode-owner";
pub const OPENCODE_ANSWER_DIR: &str = "opencode-answer";
pub const OPENCODE_PROMPT_DIR: &str = "opencode-prompt";
pub const OPENCODE_INTERRUPT_DIR: &str = "opencode-interrupt";
pub const OPENCODE_INTERRUPT_RESULT_DIR: &str = "opencode-interrupt-result";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeFiles {
    pub preflight: PathBuf,
    pub healthy: PathBuf,
    pub spawn: PathBuf,
    pub close: PathBuf,
    pub error: PathBuf,
    pub owner: PathBuf,
    pub answer: PathBuf,
    pub prompt: PathBuf,
    pub interrupt: PathBuf,
    pub interrupt_result: PathBuf,
    pub session: PathBuf,
}

impl OpencodeFiles {
    pub fn new(store: &Path, id: &str) -> Self {
        Self {
            preflight: store.join(OPENCODE_PREFLIGHT_DIR).join(id),
            healthy: store.join(OPENCODE_HEALTHY_DIR).join(id),
            spawn: store.join(OPENCODE_SPAWN_DIR).join(id),
            close: store.join(OPENCODE_CLOSE_DIR).join(id),
            error: store.join(OPENCODE_ERROR_DIR).join(id),
            owner: store.join(OPENCODE_OWNER_DIR).join(id),
            answer: store.join(OPENCODE_ANSWER_DIR).join(id),
            prompt: store.join(OPENCODE_PROMPT_DIR).join(id),
            interrupt: store.join(OPENCODE_INTERRUPT_DIR).join(id),
            interrupt_result: store.join(OPENCODE_INTERRUPT_RESULT_DIR).join(id),
            session: store.join(OPENCODE_SESSION_DIR).join(id),
        }
    }

    pub fn requests(&self) -> [&Path; 10] {
        [
            &self.preflight,
            &self.healthy,
            &self.spawn,
            &self.close,
            &self.error,
            &self.owner,
            &self.answer,
            &self.prompt,
            &self.interrupt,
            &self.interrupt_result,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpencodeSpawn {
    pub worktree: PathBuf,
    pub inbox: PathBuf,
    pub model: String,
    pub task: String,
    pub at: u64,
    pub snapshots: bool,
}

impl OpencodeSpawn {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRecord {
    pub port: u16,
    pub password: String,
    pub pid: u32,
    pub start: String,
}

impl ServerRecord {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

pub fn opencode_server_record_path(tmp: &Path, pid: u32) -> PathBuf {
    tmp.join(format!("ralph-opencode-{pid}.json"))
}

const SERVER_RECORD_PREFIX: &str = "ralph-opencode-";
const SERVER_RECORD_SUFFIX: &str = ".json";

pub fn server_record_pid(file_name: &str) -> Option<u32> {
    file_name
        .strip_prefix(SERVER_RECORD_PREFIX)?
        .strip_suffix(SERVER_RECORD_SUFFIX)?
        .parse()
        .ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerRecordFacts<'a> {
    pub file_pid: Option<u32>,
    pub own_pid: u32,
    pub record: Option<&'a ServerRecord>,
    pub file_pid_start_now: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordFate {
    Keep,
    Delete,
    Ignore,
}

pub fn server_record_fate(facts: ServerRecordFacts) -> RecordFate {
    let Some(file_pid) = facts.file_pid else {
        return RecordFate::Ignore;
    };
    if file_pid == facts.own_pid {
        return RecordFate::Keep;
    }
    let Some(start_now) = facts.file_pid_start_now else {
        return RecordFate::Delete;
    };
    match facts.record {
        Some(record) if record.pid == file_pid && record.start == start_now => RecordFate::Keep,
        Some(_) => RecordFate::Delete,
        None => RecordFate::Keep,
    }
}

pub fn server_record_usable(
    record: &ServerRecord,
    own_pid: u32,
    own_start_now: Option<&str>,
) -> bool {
    record.pid == own_pid && own_start_now == Some(record.start.as_str())
}

pub fn session_permissions() -> Value {
    json!([
        { "action": "*", "resource": "*", "effect": "allow" },
        { "action": "question", "resource": "*", "effect": "deny" },
    ])
}

pub fn permission_reply_path(session_id: &str, request_id: &str) -> String {
    format!("/api/session/{session_id}/permission/{request_id}/reply")
}

pub fn answer_decision(
    front: Option<&PermissionAsked>,
    text: &str,
) -> Result<(String, PermissionDecision), String> {
    let asked = front.ok_or("no permission request is pending for this worker")?;
    let options = opencode_blocked_content(asked).options;
    let option = match_option(&options, text).ok_or_else(|| no_option_message(&options))?;
    let decision =
        permission_decision(&option.option_id).ok_or_else(|| no_option_message(&options))?;
    Ok((asked.id.clone(), decision))
}

pub fn opencode_close_handoff(
    owner_exists: bool,
    session_exists: bool,
    elapsed: Duration,
) -> Handoff {
    close_handoff(owner_exists || session_exists, elapsed)
}

pub fn session_missing(message: &str) -> bool {
    message.contains("HTTP 404")
}

pub fn transient_request_error(message: &str) -> bool {
    message
        .strip_prefix("HTTP ")
        .and_then(|rest| rest.get(..3))
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|status| matches!(status, 404 | 408 | 429 | 500..=599))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_live_under_their_own_directories() {
        let files = OpencodeFiles::new(Path::new("/s"), "GUZ-1");
        assert_eq!(
            files.preflight,
            PathBuf::from("/s/opencode-preflight/GUZ-1")
        );
        assert_eq!(files.healthy, PathBuf::from("/s/opencode-healthy/GUZ-1"));
        assert_eq!(files.spawn, PathBuf::from("/s/opencode-spawn/GUZ-1"));
        assert_eq!(files.close, PathBuf::from("/s/opencode-close/GUZ-1"));
        assert_eq!(files.error, PathBuf::from("/s/opencode-error/GUZ-1"));
        assert_eq!(files.owner, PathBuf::from("/s/opencode-owner/GUZ-1"));
        assert_eq!(files.answer, PathBuf::from("/s/opencode-answer/GUZ-1"));
        assert_eq!(files.prompt, PathBuf::from("/s/opencode-prompt/GUZ-1"));
        assert_eq!(
            files.interrupt,
            PathBuf::from("/s/opencode-interrupt/GUZ-1")
        );
        assert_eq!(files.session, PathBuf::from("/s/opencode-session/GUZ-1"));
    }

    #[test]
    fn requests_leave_out_the_session_record() {
        let files = OpencodeFiles::new(Path::new("/s"), "A");
        assert!(!files.requests().contains(&files.session.as_path()));
        assert_eq!(files.requests().len(), 10);
    }

    #[test]
    fn spawn_request_round_trips_and_rejects_garbage() {
        let request = OpencodeSpawn {
            worktree: PathBuf::from("/w"),
            inbox: PathBuf::from("/i"),
            model: "github-copilot/claude-haiku-4.5".to_string(),
            task: "do \"it\"\nnow".to_string(),
            at: 7,
            snapshots: true,
        };
        assert_eq!(OpencodeSpawn::parse(&request.render()), Some(request));
        assert_eq!(OpencodeSpawn::parse("nope"), None);
    }

    #[test]
    fn server_record_round_trips_and_rejects_garbage() {
        let record = record(7, "Mon Jan  1 00:00:00 2026");
        assert_eq!(ServerRecord::parse(&record.render()), Some(record));
        assert_eq!(ServerRecord::parse(""), None);
    }

    #[test]
    fn server_record_without_owner_fields_fails_to_parse() {
        assert_eq!(
            ServerRecord::parse(r#"{"port":4096,"password":"abc"}"#),
            None
        );
    }

    fn record(pid: u32, start: &str) -> ServerRecord {
        ServerRecord {
            port: 4096,
            password: "abc".to_string(),
            pid,
            start: start.to_string(),
        }
    }

    fn fate(
        file_pid: Option<u32>,
        record: Option<&ServerRecord>,
        start_now: Option<&str>,
    ) -> RecordFate {
        server_record_fate(ServerRecordFacts {
            file_pid,
            own_pid: 1,
            record,
            file_pid_start_now: start_now,
        })
    }

    #[test]
    fn server_record_pid_is_strict() {
        assert_eq!(server_record_pid("ralph-opencode-42.json"), Some(42));
        assert_eq!(server_record_pid("ralph-opencode-x.json"), None);
        assert_eq!(server_record_pid("ralph-opencode-1.txt"), None);
        assert_eq!(server_record_pid("ralph-opencode-.json"), None);
        assert_eq!(server_record_pid("other.json"), None);
    }

    #[test]
    fn fate_ignores_malformed_names() {
        assert_eq!(fate(None, None, None), RecordFate::Ignore);
    }

    #[test]
    fn fate_keeps_own_record() {
        assert_eq!(fate(Some(1), None, None), RecordFate::Keep);
    }

    #[test]
    fn fate_keeps_record_of_live_owner() {
        let rec = record(5, "s");
        assert_eq!(fate(Some(5), Some(&rec), Some("s")), RecordFate::Keep);
    }

    #[test]
    fn fate_deletes_record_of_dead_owner() {
        let rec = record(5, "s");
        assert_eq!(fate(Some(5), Some(&rec), None), RecordFate::Delete);
        assert_eq!(fate(Some(5), None, None), RecordFate::Delete);
    }

    #[test]
    fn fate_deletes_record_of_reused_pid() {
        let rec = record(5, "old");
        assert_eq!(fate(Some(5), Some(&rec), Some("new")), RecordFate::Delete);
        let other = record(6, "new");
        assert_eq!(fate(Some(5), Some(&other), Some("new")), RecordFate::Delete);
    }

    #[test]
    fn fate_keeps_unreadable_record_of_live_pid() {
        assert_eq!(fate(Some(5), None, Some("s")), RecordFate::Keep);
    }

    #[test]
    fn usable_needs_matching_pid_and_start() {
        let rec = record(9, "s");
        assert!(server_record_usable(&rec, 9, Some("s")));
        assert!(!server_record_usable(&rec, 8, Some("s")));
        assert!(!server_record_usable(&rec, 9, Some("t")));
        assert!(!server_record_usable(&rec, 9, None));
    }

    #[test]
    fn server_record_path_is_keyed_by_the_chat_pid() {
        assert_eq!(
            opencode_server_record_path(Path::new("/tmp"), 42),
            PathBuf::from("/tmp/ralph-opencode-42.json")
        );
    }

    #[test]
    fn permissions_allow_everything_then_deny_questions_last() {
        let rules = session_permissions();
        let rules = rules.as_array().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["action"], "*");
        assert_eq!(rules[0]["effect"], "allow");
        assert_eq!(rules[1]["action"], "question");
        assert_eq!(rules[1]["effect"], "deny");
    }

    fn asked() -> PermissionAsked {
        PermissionAsked {
            id: "per_1".to_string(),
            session_id: "ses_1".to_string(),
            action: "shell".to_string(),
            resources: vec!["touch x".to_string()],
            save: Vec::new(),
        }
    }

    #[test]
    fn permission_reply_path_names_the_session_and_request() {
        assert_eq!(
            permission_reply_path("ses_1", "per_1"),
            "/api/session/ses_1/permission/per_1/reply"
        );
    }

    #[test]
    fn answer_decision_maps_an_option_id_to_the_front_request() {
        let asked = asked();
        assert_eq!(
            answer_decision(Some(&asked), " always \n"),
            Ok(("per_1".to_string(), PermissionDecision::Always))
        );
        assert_eq!(
            answer_decision(Some(&asked), "once"),
            Ok(("per_1".to_string(), PermissionDecision::Once))
        );
        assert_eq!(
            answer_decision(Some(&asked), "reject"),
            Ok(("per_1".to_string(), PermissionDecision::Reject))
        );
    }

    #[test]
    fn answer_decision_refuses_unknown_options_and_empty_queues() {
        let asked = asked();
        let unknown = answer_decision(Some(&asked), "yes").unwrap_err();
        assert!(unknown.contains("once, always, reject"), "{unknown}");
        assert_eq!(
            answer_decision(None, "once").unwrap_err(),
            "no permission request is pending for this worker"
        );
    }

    #[test]
    fn opencode_close_waits_for_both_owner_and_session_record() {
        let early = Duration::from_secs(1);
        assert_eq!(opencode_close_handoff(true, true, early), Handoff::Pending);
        assert_eq!(opencode_close_handoff(false, true, early), Handoff::Pending);
        assert_eq!(opencode_close_handoff(true, false, early), Handoff::Pending);
        assert_eq!(opencode_close_handoff(false, false, early), Handoff::Done);
    }

    #[test]
    fn opencode_close_times_out_while_either_file_remains() {
        let late = crate::worker::HANDOFF_TIMEOUT;
        assert_eq!(opencode_close_handoff(false, true, late), Handoff::TimedOut);
        assert_eq!(opencode_close_handoff(true, false, late), Handoff::TimedOut);
        assert_eq!(opencode_close_handoff(false, false, late), Handoff::Done);
    }

    #[test]
    fn session_missing_matches_a_404_only() {
        assert!(session_missing(
            r#"HTTP 404: {"_tag":"SessionNotFoundError"}"#
        ));
        assert!(!session_missing("HTTP 500: boom"));
    }

    #[test]
    fn transient_request_errors_are_retryable_statuses_only() {
        assert!(transient_request_error("HTTP 404: not found"));
        assert!(transient_request_error("HTTP 503: busy"));
        assert!(transient_request_error("HTTP 429: slow down"));
        assert!(!transient_request_error("HTTP 400: bad"));
        assert!(!transient_request_error("HTTP 409: exists"));
        assert!(!transient_request_error("connection refused"));
        assert!(!transient_request_error("HTTP "));
    }
}
