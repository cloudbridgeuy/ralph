//! Test helpers for output block serialization tests.
//!
//! This module provides utilities for testing OutputBlock serialization,
//! reducing boilerplate in tests that verify TOML round-trip behavior.

use ralph_core::stream::ToolInvocation;

use ralph_core::stream::blocks::{OutputBlock, ToolInvocationBlock};

/// Create a ToolInvocation for testing.
///
/// This helper creates a ToolInvocation with the given name and input JSON,
/// reducing the boilerplate of constructing test invocations.
///
/// # Example
///
/// ```ignore
/// use crate::stream_processor::test_helpers::create_test_invocation;
/// use serde_json::json;
///
/// let invocation = create_test_invocation("Bash", json!({
///     "command": "ls -la",
///     "description": "List files"
/// }));
/// ```
pub fn create_test_invocation(name: &str, input: serde_json::Value) -> ToolInvocation {
    ToolInvocation {
        id: "test-id".to_string(),
        name: name.to_string(),
        input,
    }
}

/// Assert that an OutputBlock is a ToolInvocation and return the inner block.
///
/// Panics with a descriptive message if the block is not a ToolInvocation.
///
/// # Example
///
/// ```ignore
/// use crate::stream_processor::test_helpers::expect_tool_invocation;
///
/// let block = build_tool_invocation_block(&invocation);
/// let inv = expect_tool_invocation(block);
/// assert_eq!(inv.tool_name, "Bash");
/// ```
pub fn expect_tool_invocation(block: OutputBlock) -> ToolInvocationBlock {
    match block {
        OutputBlock::ToolInvocation(inv) => inv,
        other => panic!(
            "Expected ToolInvocation variant, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}
