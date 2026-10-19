use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorFiles {
    pub spawn: PathBuf,
    pub close: PathBuf,
    pub error: PathBuf,
    pub owner: PathBuf,
    pub answer: PathBuf,
    pub prompt: PathBuf,
    pub interrupt: PathBuf,
    pub session: PathBuf,
    pub interrupt_result: PathBuf,
}

impl CursorFiles {
    pub fn new(store: &Path, id: &str) -> Self {
        Self {
            spawn: store.join(CURSOR_SPAWN_DIR).join(id),
            close: store.join(CURSOR_CLOSE_DIR).join(id),
            error: store.join(CURSOR_ERROR_DIR).join(id),
            owner: store.join(CURSOR_OWNER_DIR).join(id),
            answer: store.join(CURSOR_ANSWER_DIR).join(id),
            prompt: store.join(CURSOR_PROMPT_DIR).join(id),
            interrupt: store.join(CURSOR_INTERRUPT_DIR).join(id),
            session: store.join(CURSOR_SESSION_DIR).join(id),
            interrupt_result: store.join(CURSOR_INTERRUPT_RESULT_DIR).join(id),
        }
    }

    pub fn all(&self) -> [&Path; 8] {
        [
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
pub struct SpawnRequest {
    pub worktree: PathBuf,
    pub exe: PathBuf,
    pub inbox: PathBuf,
    pub task: String,
    pub at: u64,
    pub snapshots: bool,
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
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(&text).unwrap(),
            request
        );
    }
}
