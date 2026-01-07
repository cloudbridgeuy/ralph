//! Ralph Core Library
//!
//! This crate contains pure business logic for the ralph CLI tool.
//! Following the Functional Core - Imperative Shell pattern, all functions
//! here are pure transformations without I/O side effects.

pub mod chunk;
pub mod completion;
pub mod context;
pub mod prd;
pub mod session;
pub mod stream;
