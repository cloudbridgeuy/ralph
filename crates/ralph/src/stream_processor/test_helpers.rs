//! Test helpers for output block serialization tests.
//!
//! This module provides utilities for testing OutputBlock serialization,
//! reducing boilerplate in tests that verify TOML round-trip behavior.

use ralph_core::stream::ToolInvocation;
use serde::{de::DeserializeOwned, Serialize};

use super::output_block::{OutputBlock, ToolInvocationBlock};

/// Assert that a value can round-trip through TOML serialization.
///
/// This helper serializes the value to TOML, deserializes it back,
/// and asserts equality with the original.
///
/// # Example
///
/// ```ignore
/// use crate::stream_processor::test_helpers::assert_toml_roundtrip;
/// use crate::stream_processor::OutputBlock;
///
/// let block = OutputBlock::separator();
/// assert_toml_roundtrip(&block);
/// ```
pub fn assert_toml_roundtrip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let serialized = toml::to_string(value).expect("Failed to serialize to TOML");
    let deserialized: T = toml::from_str(&serialized).expect("Failed to deserialize from TOML");
    assert_eq!(
        value, &deserialized,
        "Round-trip serialization failed.\nOriginal: {:?}\nDeserialized: {:?}\nSerialized TOML:\n{}",
        value, deserialized, serialized
    );
}

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
