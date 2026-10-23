mod approvals;
mod close;
mod protocol;
mod rpc;
mod turns;

pub use approvals::*;
pub use close::*;
pub use protocol::*;
pub use rpc::*;
pub use turns::*;

pub const CODEX_COMMAND: &str = "codex";
pub const CODEX_SPAWN_DIR: &str = "codex-spawn";
pub const CODEX_CLOSE_DIR: &str = "codex-close";
pub const CODEX_ERROR_DIR: &str = "codex-error";
pub const CODEX_OWNER_DIR: &str = "codex";
pub const CODEX_ANSWER_DIR: &str = "codex-answer";
pub const CODEX_PROMPT_DIR: &str = "codex-prompt";
pub const CODEX_INTERRUPT_DIR: &str = "codex-interrupt";
pub const CODEX_INTERRUPT_RESULT_DIR: &str = "codex-interrupt-result";
pub const CODEX_SESSION_DIR: &str = "codex-session";
