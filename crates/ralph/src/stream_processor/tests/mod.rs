//! Tests for the stream processor module.
//!
//! This module organizes tests into logical groups covering different
//! aspects of stream processing functionality.

// Core functionality tests
mod core;

// Whitespace preservation tests
mod whitespace;

// Tool display formatting tests
mod tool_display;

// Visual separation between responses
mod visual_separation;

// Path display in tool invocations
mod path_display;

// Edit diff result formatting
mod edit_diff;

// Bash command rendering
mod bash_rendering;

// Verbose tools configuration
mod verbose_config;

// Grep verbose mode
mod grep_verbose;
