use crate::stream::ToolInvocation;
use serde::{de::DeserializeOwned, Serialize};

use super::{OutputBlock, ToolInvocationBlock};

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

pub fn create_test_invocation(name: &str, input: serde_json::Value) -> ToolInvocation {
    ToolInvocation {
        id: "test-id".to_string(),
        name: name.to_string(),
        input,
    }
}

pub fn expect_tool_invocation(block: OutputBlock) -> ToolInvocationBlock {
    match block {
        OutputBlock::ToolInvocation(inv) => inv,
        other => panic!(
            "Expected ToolInvocation variant, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}
