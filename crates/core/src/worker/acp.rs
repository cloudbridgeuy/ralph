use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod agy_model;
mod agy_notice;
mod agy_stream;
mod agy_usage;
mod frames;
mod handoff;
mod log;
mod owner;
mod permissions;
mod protocol;
mod turns;

pub use agy_model::*;
pub use agy_notice::*;
pub use agy_stream::*;
pub use agy_usage::*;
pub use frames::*;
pub use handoff::*;
pub use log::*;
pub use owner::*;
pub use permissions::*;
pub use protocol::*;
pub use turns::*;

pub const AGY_OWNER_DIR: &str = "agy";

pub const CURSOR_HARNESS: &str = "cursor";
pub const CURSOR_SPAWN_DIR: &str = "cursor-spawn";
pub const CURSOR_CLOSE_DIR: &str = "cursor-close";
pub const CURSOR_ERROR_DIR: &str = "cursor-error";
pub const CURSOR_OWNER_DIR: &str = "cursor";
pub const CURSOR_ANSWER_DIR: &str = "cursor-answer";
pub const CURSOR_PROMPT_DIR: &str = "cursor-prompt";
pub const CURSOR_INTERRUPT_DIR: &str = "cursor-interrupt";
pub const CURSOR_SESSION_DIR: &str = "cursor-session";
pub const CURSOR_INTERRUPT_RESULT_DIR: &str = "cursor-interrupt-result";
pub(crate) const REPORT_TOOL: &str = "report";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRequest {
    pub worktree: PathBuf,
    pub exe: PathBuf,
    pub inbox: PathBuf,
    pub task: String,
    pub at: u64,
    pub snapshots: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scheduled: bool,
}

impl SpawnRequest {
    pub fn prompt(&self) -> String {
        task_prompt(&self.task, self.scheduled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_request_round_trips_through_json() {
        let request = SpawnRequest {
            worktree: "/w".into(),
            exe: "/bin/ralph".into(),
            inbox: "/s/inbox".into(),
            task: "do it".into(),
            at: 7,
            snapshots: true,
            model: Some("gpt-5".into()),
            scheduled: false,
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(&text).unwrap(),
            request
        );
        let bare = r#"{"worktree":"/w","exe":"/bin/ralph","inbox":"/s/inbox","task":"do it","at":7,"snapshots":false}"#;
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(bare).unwrap().model,
            None
        );
    }

    #[test]
    fn spawn_request_prompt_follows_the_scheduled_flag() {
        let bare = r#"{"worktree":"/w","exe":"/bin/ralph","inbox":"/s/inbox","task":"do it","at":7,"snapshots":false}"#;
        let mut request = serde_json::from_str::<SpawnRequest>(bare).unwrap();
        assert_eq!(request.prompt(), task_prompt("do it", false));
        request.scheduled = true;
        assert_eq!(request.prompt(), task_prompt("do it", true));
    }

    #[test]
    fn spawn_request_scheduled_defaults_to_false_and_round_trips() {
        let bare = r#"{"worktree":"/w","exe":"/bin/ralph","inbox":"/s/inbox","task":"do it","at":7,"snapshots":false}"#;
        let mut request = serde_json::from_str::<SpawnRequest>(bare).unwrap();
        assert!(!request.scheduled);
        assert!(!serde_json::to_string(&request)
            .unwrap()
            .contains("scheduled"));
        request.scheduled = true;
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains(r#""scheduled":true"#));
        assert!(
            serde_json::from_str::<SpawnRequest>(&text)
                .unwrap()
                .scheduled
        );
    }
}
