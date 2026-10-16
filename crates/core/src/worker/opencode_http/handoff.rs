use super::{
    opencode_blocked_content, permission_decision, PermissionAsked, PermissionDecision,
    OPENCODE_SESSION_DIR,
};
use crate::worker::{match_option, no_option_message};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const OPENCODE_SPAWN_DIR: &str = "opencode-spawn";
pub const OPENCODE_CLOSE_DIR: &str = "opencode-close";
pub const OPENCODE_ERROR_DIR: &str = "opencode-error";
pub const OPENCODE_OWNER_DIR: &str = "opencode-owner";
pub const OPENCODE_ANSWER_DIR: &str = "opencode-answer";
pub const OPENCODE_PROMPT_DIR: &str = "opencode-prompt";
pub const OPENCODE_INTERRUPT_DIR: &str = "opencode-interrupt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpencodeFiles {
    pub spawn: PathBuf,
    pub close: PathBuf,
    pub error: PathBuf,
    pub owner: PathBuf,
    pub answer: PathBuf,
    pub prompt: PathBuf,
    pub interrupt: PathBuf,
    pub session: PathBuf,
}

impl OpencodeFiles {
    pub fn new(store: &Path, id: &str) -> Self {
        Self {
            spawn: store.join(OPENCODE_SPAWN_DIR).join(id),
            close: store.join(OPENCODE_CLOSE_DIR).join(id),
            error: store.join(OPENCODE_ERROR_DIR).join(id),
            owner: store.join(OPENCODE_OWNER_DIR).join(id),
            answer: store.join(OPENCODE_ANSWER_DIR).join(id),
            prompt: store.join(OPENCODE_PROMPT_DIR).join(id),
            interrupt: store.join(OPENCODE_INTERRUPT_DIR).join(id),
            session: store.join(OPENCODE_SESSION_DIR).join(id),
        }
    }

    pub fn requests(&self) -> [&Path; 7] {
        [
            &self.spawn,
            &self.close,
            &self.error,
            &self.owner,
            &self.answer,
            &self.prompt,
            &self.interrupt,
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
        assert_eq!(files.requests().len(), 7);
    }

    #[test]
    fn spawn_request_round_trips_and_rejects_garbage() {
        let request = OpencodeSpawn {
            worktree: PathBuf::from("/w"),
            inbox: PathBuf::from("/i"),
            model: "github-copilot/claude-haiku-4.5".to_string(),
            task: "do \"it\"\nnow".to_string(),
            at: 7,
        };
        assert_eq!(OpencodeSpawn::parse(&request.render()), Some(request));
        assert_eq!(OpencodeSpawn::parse("nope"), None);
    }

    #[test]
    fn server_record_round_trips_and_rejects_garbage() {
        let record = ServerRecord {
            port: 4096,
            password: "abc".to_string(),
        };
        assert_eq!(ServerRecord::parse(&record.render()), Some(record));
        assert_eq!(ServerRecord::parse(""), None);
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
