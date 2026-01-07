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
fn test_extract_tool_invocations_from_events_single_assistant() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/test.rs"}),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let invocations = extract_tool_invocations_from_events(&events);
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].name, "Read");
}

#[test]
fn test_extract_tool_invocations_from_events_multiple_assistants() {
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({"pattern": "*.rs"}),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/src/main.rs"}),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    let invocations = extract_tool_invocations_from_events(&events);
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].name, "Glob");
    assert_eq!(invocations[0].id, "toolu_01");
    assert_eq!(invocations[1].name, "Read");
    assert_eq!(invocations[1].id, "toolu_02");
}

#[test]
fn test_extract_tool_invocations_from_events_mixed_event_types() {
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("abc".to_string()),
            model: None,
            tools: vec![],
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![
                    ContentBlock::Text {
                        text: "Searching...".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01".to_string(),
                        name: "Glob".to_string(),
                        input: serde_json::json!({"pattern": "*.rs"}),
                    },
                ],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_01".to_string()),
                    content: Some("/src/main.rs".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({"file_path": "/src/main.rs"}),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Result(ResultEvent {
            subtype: Some("success".to_string()),
            total_cost_usd: Some(0.01),
            cost_usd: None,
            duration_ms: Some(1000),
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        }),
    ];

    let invocations = extract_tool_invocations_from_events(&events);
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].name, "Glob");
    assert_eq!(invocations[1].name, "Read");
}

#[test]
fn test_extract_tool_invocations_from_events_empty() {
    let events: Vec<StreamEvent> = vec![];
    let invocations = extract_tool_invocations_from_events(&events);
    assert!(invocations.is_empty());
}

#[test]
fn test_extract_tool_invocations_from_events_no_assistant() {
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: None,
            session_id: None,
            model: None,
            tools: vec![],
        }),
        StreamEvent::Result(ResultEvent {
            subtype: None,
            total_cost_usd: None,
            cost_usd: None,
            duration_ms: None,
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        }),
    ];

    let invocations = extract_tool_invocations_from_events(&events);
    assert!(invocations.is_empty());
}

#[test]
fn test_extract_tool_invocations_from_events_text_only_assistants() {
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "Done!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    let invocations = extract_tool_invocations_from_events(&events);
    assert!(invocations.is_empty());
}

#[test]
fn test_extract_tool_invocations_preserves_order() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![
                ContentBlock::ToolUse {
                    id: "toolu_a".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_b".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_c".to_string(),
                    name: "Edit".to_string(),
                    input: serde_json::json!({}),
                },
            ],
            model: None,
            stop_reason: None,
        },
    })];

    let invocations = extract_tool_invocations_from_events(&events);
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].id, "toolu_a");
    assert_eq!(invocations[1].id, "toolu_b");
    assert_eq!(invocations[2].id, "toolu_c");
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
