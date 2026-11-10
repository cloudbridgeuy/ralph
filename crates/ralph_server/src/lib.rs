#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub(crate) mod acp_log;
pub(crate) mod acp_worker;
pub mod chat;
pub(crate) mod claude_cleanup;
pub(crate) mod claude_worker;
pub(crate) mod codex_usage;
pub(crate) mod codex_worker;
pub mod config;
pub(crate) mod cursor_cleanup;
pub mod cursor_owner;
pub(crate) mod cursor_sweep;
pub(crate) mod cursor_usage;
pub mod hook_bridge;
pub(crate) mod json_file;
pub mod ledger;
pub(crate) mod opencode_usage;
pub mod opencode_worker;
pub(crate) mod outcome_log;
pub mod paths;
pub mod permission_queue;
pub mod private_file;
pub(crate) mod process_tree;
pub mod snapshot;
pub mod subprocess;
pub(crate) mod transport;
pub(crate) mod verified_agents;
pub mod version_drift;
pub mod warn;
pub mod watch_store;
pub mod watches;
pub mod wire;
pub mod worker_changes;
pub mod worker_events;
pub mod worker_mcp;
pub mod worker_usage;
pub mod worker_verify;
pub mod workers;
