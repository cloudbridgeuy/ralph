use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod frames;
mod handoff;
mod log;
mod owner;
mod permissions;
mod protocol;
mod turns;

pub use frames::*;
pub use handoff::*;
pub use log::*;
pub use owner::*;
pub use permissions::*;
pub use protocol::*;
pub use turns::*;

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
const REPORT_TOOL: &str = "report";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast: Option<bool>,
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
            reasoning_effort: Some("low".into()),
            fast: Some(false),
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
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(bare)
                .unwrap()
                .reasoning_effort,
            None
        );
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(bare).unwrap().fast,
            None
        );
    }
}
