#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub mod acp_log;
pub mod acp_worker;
pub mod chat;
pub mod claude_cleanup;
pub mod claude_headless_worker;
pub mod codex_usage;
pub mod codex_worker;
pub mod config;
pub mod cursor_cleanup;
pub mod cursor_owner;
pub mod cursor_sweep;
pub mod cursor_usage;
pub mod hook_bridge;
pub mod json_file;
pub mod ledger;
pub mod opencode_usage;
pub mod opencode_worker;
pub mod outcome_log;
pub mod paths;
pub mod permission_queue;
pub mod private_file;
pub mod process_tree;
pub mod snapshot;
pub mod subprocess;
pub mod transport;
pub mod verified_agents;
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
