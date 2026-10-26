//! Ralph Core Library
//!
//! This crate contains pure business logic for the ralph CLI tool.
//! Following the Functional Core - Imperative Shell pattern, all functions
//! here are pure transformations without I/O side effects.

#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub mod attachment;
pub mod changes;
pub mod chat;
pub mod chat_cell;
pub mod chat_draft;
pub mod chat_edit_mode;
pub mod chat_help;
pub mod chat_hold;
pub mod chat_input;
pub mod chat_memory;
pub mod chat_skills;
pub mod chat_socket;
pub mod chat_status;
pub mod chat_suggestion;
pub mod chat_thread;
pub mod chat_typed;
pub mod chunk;
pub mod claude_leftovers;
pub mod cursor_leftovers;
pub mod diff_command;
pub mod directive;
pub mod inbox;
pub mod inbox_alert;
pub mod kitty;
pub mod mcp;
pub mod retry;
pub mod sanitize;
pub mod schedule;
pub mod schedule_command;
pub mod schedule_request;
pub mod semver;
pub mod shell_format;
pub mod snapshot;
pub mod status_mode;
pub mod status_queue;
pub mod status_segments;
pub mod stream;
pub mod watch;
pub mod watch_command;
pub mod watch_request;
pub mod watch_store;
pub mod wire;
pub mod worker;
pub mod worker_check;
pub mod workers_view;
