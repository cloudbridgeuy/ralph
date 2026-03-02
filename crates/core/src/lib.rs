//! Ralph Core Library
//!
//! This crate contains pure business logic for the ralph CLI tool.
//! Following the Functional Core - Imperative Shell pattern, all functions
//! here are pure transformations without I/O side effects.

// Deny .unwrap() and .expect() in non-test code to ensure proper error handling.
// Test code may still use them for brevity.
// Any intentional uses must be documented with #[allow(...)] and comments.
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
// Functions should have at most 5 arguments. Use config/options structs for more.
// Threshold configured in clippy.toml (too-many-arguments-threshold = 5).
#![cfg_attr(not(test), deny(clippy::too_many_arguments))]

pub mod chunk;
pub mod completion;
pub mod context;
pub mod paused;
pub mod prd;
pub mod session;
pub mod stream;
