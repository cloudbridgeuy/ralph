use crate::stream::*;

#[test]
fn test_assistant_message_extract_tool_invocations_single() {
    let message = AssistantMessage {
        id: Some("msg_01".to_string()),
        content: vec![ContentBlock::ToolUse {
            id: "toolu_01".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({"file_path": "/src/main.rs"}),
        }],
        model: None,
        stop_reason: None,
    };

    let invocations = message.extract_tool_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].id, "toolu_01");
    assert_eq!(invocations[0].name, "Read");
    assert_eq!(invocations[0].input["file_path"], "/src/main.rs");
}

#[test]
fn test_assistant_message_extract_tool_invocations_multiple() {
    let message = AssistantMessage {
        id: Some("msg_01".to_string()),
        content: vec![
            ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: serde_json::json!({"pattern": "*.rs"}),
            },
            ContentBlock::ToolUse {
                id: "toolu_02".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/src/lib.rs"}),
            },
        ],
        model: None,
        stop_reason: None,
    };

    let invocations = message.extract_tool_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].id, "toolu_01");
    assert_eq!(invocations[0].name, "Glob");
    assert_eq!(invocations[1].id, "toolu_02");
    assert_eq!(invocations[1].name, "Read");
}

#[test]
fn test_assistant_message_extract_tool_invocations_mixed_content() {
    let message = AssistantMessage {
        id: None,
        content: vec![
            ContentBlock::Text {
                text: "Let me search for files.".to_string(),
            },
            ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: serde_json::json!({"pattern": "**/*.rs"}),
            },
            ContentBlock::Text {
                text: "Now reading the file.".to_string(),
            },
            ContentBlock::ToolUse {
                id: "toolu_02".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/test.rs"}),
            },
        ],
        model: None,
        stop_reason: None,
    };

    let invocations = message.extract_tool_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].name, "Glob");
    assert_eq!(invocations[1].name, "Read");
}

#[test]
fn test_assistant_message_extract_tool_invocations_text_only() {
    let message = AssistantMessage {
        id: None,
        content: vec![
            ContentBlock::Text {
                text: "Hello, world!".to_string(),
            },
            ContentBlock::Text {
                text: " More text.".to_string(),
            },
        ],
        model: None,
        stop_reason: None,
    };

    let invocations = message.extract_tool_invocations();
    assert!(invocations.is_empty());
}

#[test]
fn test_assistant_message_extract_tool_invocations_empty_content() {
    let message = AssistantMessage {
        id: None,
        content: vec![],
        model: None,
        stop_reason: None,
    };

    let invocations = message.extract_tool_invocations();
    assert!(invocations.is_empty());
}

#[test]
fn test_assistant_message_extract_tool_invocations_preserves_input() {
    let complex_input = serde_json::json!({
        "file_path": "/src/main.rs",
        "old_string": "fn main() {}",
        "new_string": "fn main() { println!(\"Hello!\"); }",
        "nested": {
            "key": "value",
            "array": [1, 2, 3]
        }
    });

    let message = AssistantMessage {
        id: None,
        content: vec![ContentBlock::ToolUse {
            id: "toolu_edit".to_string(),
            name: "Edit".to_string(),
            input: complex_input.clone(),
        }],
        model: None,
        stop_reason: None,
    };

    let invocations = message.extract_tool_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].input, complex_input);
    assert_eq!(invocations[0].input["nested"]["array"][1], 2);
}

#[test]
fn test_assistant_event_extract_tool_invocations() {
    let event = AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: serde_json::json!({"pattern": "*.rs"}),
            }],
            model: None,
            stop_reason: None,
        },
    };

    let invocations = event.extract_tool_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].name, "Glob");
}

#[test]
fn test_tool_invocation_serialization_round_trip() {
    let invocation = ToolInvocation {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({"file_path": "/src/main.rs", "limit": 100}),
    };

    let json = serde_json::to_string(&invocation).unwrap();
    let deserialized: ToolInvocation = serde_json::from_str(&json).unwrap();
    assert_eq!(invocation, deserialized);
}
