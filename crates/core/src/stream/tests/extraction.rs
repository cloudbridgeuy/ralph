use crate::stream::*;

#[test]
fn test_assistant_message_extract_text_single_content() {
    let message = AssistantMessage {
        id: Some("msg_01".to_string()),
        content: vec![ContentBlock::Text {
            text: "Hello, world!".to_string(),
        }],
        model: None,
        stop_reason: None,
    };

    assert_eq!(message.extract_text(), "Hello, world!");
}

#[test]
fn test_assistant_message_extract_text_multi_content() {
    let message = AssistantMessage {
        id: None,
        content: vec![
            ContentBlock::Text {
                text: "First part. ".to_string(),
            },
            ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/test.rs"}),
            },
            ContentBlock::Text {
                text: "Second part.".to_string(),
            },
        ],
        model: None,
        stop_reason: None,
    };

    assert_eq!(message.extract_text(), "First part. Second part.");
}

#[test]
fn test_assistant_message_extract_text_tool_only() {
    let message = AssistantMessage {
        id: None,
        content: vec![
            ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: serde_json::json!({"pattern": "*.rs"}),
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

    assert_eq!(message.extract_text(), "");
}

#[test]
fn test_assistant_message_extract_text_empty_content() {
    let message = AssistantMessage {
        id: None,
        content: vec![],
        model: None,
        stop_reason: None,
    };

    assert_eq!(message.extract_text(), "");
}

#[test]
fn test_assistant_message_extract_text_preserves_ordering() {
    let message = AssistantMessage {
        id: None,
        content: vec![
            ContentBlock::Text {
                text: "A".to_string(),
            },
            ContentBlock::Text {
                text: "B".to_string(),
            },
            ContentBlock::Text {
                text: "C".to_string(),
            },
        ],
        model: None,
        stop_reason: None,
    };

    assert_eq!(message.extract_text(), "ABC");
}

#[test]
fn test_assistant_event_extract_text() {
    let event = AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Hello from assistant event!".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    };

    assert_eq!(event.extract_text(), "Hello from assistant event!");
}

#[test]
fn test_extract_text_from_events_assistant_only() {
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "First.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: None,
                content: vec![ContentBlock::Text {
                    text: "Second.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    assert_eq!(extract_text_from_events(&events), "First.Second.");
}

#[test]
fn test_extract_text_from_events_mixed() {
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
                content: vec![ContentBlock::Text {
                    text: "Hello! ".to_string(),
                }],
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
                    content: Some("file contents".to_string()),
                    is_error: false,
                }],
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

    assert_eq!(extract_text_from_events(&events), "Hello! Done!");
}

#[test]
fn test_extract_text_from_events_empty() {
    let events: Vec<StreamEvent> = vec![];
    assert_eq!(extract_text_from_events(&events), "");
}

#[test]
fn test_extract_text_from_events_no_assistant() {
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

    assert_eq!(extract_text_from_events(&events), "");
}

#[test]
fn test_extract_text_from_events_tool_only_assistant() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({}),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    assert_eq!(extract_text_from_events(&events), "");
}

// ==========================================================================
// IterationMetadata extraction tests (Story #17)
// ==========================================================================

#[test]
fn test_iteration_metadata_new() {
    let metadata = IterationMetadata::new();
    assert!(metadata.is_empty());
    assert_eq!(metadata.session_id, None);
    assert_eq!(metadata.model, None);
    assert!(metadata.tools.is_empty());
}

#[test]
fn test_iteration_metadata_is_empty() {
    let empty = IterationMetadata::default();
    assert!(empty.is_empty());

    let with_session_id = IterationMetadata {
        session_id: Some("abc".to_string()),
        ..Default::default()
    };
    assert!(!with_session_id.is_empty());

    let with_model = IterationMetadata {
        model: Some("claude".to_string()),
        ..Default::default()
    };
    assert!(!with_model.is_empty());

    let with_tools = IterationMetadata {
        tools: vec![Tool {
            name: "Read".to_string(),
            description: None,
        }],
        ..Default::default()
    };
    assert!(!with_tools.is_empty());
}

#[test]
fn test_system_event_is_init() {
    let init_event = SystemEvent {
        subtype: Some("init".to_string()),
        session_id: None,
        model: None,
        tools: vec![],
    };
    assert!(init_event.is_init());

    let other_event = SystemEvent {
        subtype: Some("other".to_string()),
        session_id: None,
        model: None,
        tools: vec![],
    };
    assert!(!other_event.is_init());

    let no_subtype = SystemEvent {
        subtype: None,
        session_id: None,
        model: None,
        tools: vec![],
    };
    assert!(!no_subtype.is_init());
}

#[test]
fn test_system_event_extract_metadata() {
    let event = SystemEvent {
        subtype: Some("init".to_string()),
        session_id: Some("session-123".to_string()),
        model: Some("claude-opus-4-5-20251101".to_string()),
        tools: vec![
            Tool {
                name: "Read".to_string(),
                description: Some("Read files".to_string()),
            },
            Tool {
                name: "Edit".to_string(),
                description: None,
            },
        ],
    };

    let metadata = event.extract_metadata();
    assert_eq!(metadata.session_id, Some("session-123".to_string()));
    assert_eq!(metadata.model, Some("claude-opus-4-5-20251101".to_string()));
    assert_eq!(metadata.tools.len(), 2);
    assert_eq!(metadata.tools[0].name, "Read");
    assert_eq!(
        metadata.tools[0].description,
        Some("Read files".to_string())
    );
    assert_eq!(metadata.tools[1].name, "Edit");
    assert_eq!(metadata.tools[1].description, None);
}

#[test]
fn test_system_event_extract_metadata_missing_fields() {
    let event = SystemEvent {
        subtype: Some("init".to_string()),
        session_id: None,
        model: None,
        tools: vec![],
    };

    let metadata = event.extract_metadata();
    assert_eq!(metadata.session_id, None);
    assert_eq!(metadata.model, None);
    assert!(metadata.tools.is_empty());
    assert!(metadata.is_empty());
}

#[test]
fn test_extract_metadata_from_events_init_present() {
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("f5b6aaac-4316-454a".to_string()),
            model: Some("claude-opus-4-5-20251101".to_string()),
            tools: vec![
                Tool {
                    name: "Glob".to_string(),
                    description: Some("Find files".to_string()),
                },
                Tool {
                    name: "Read".to_string(),
                    description: None,
                },
            ],
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_some());
    let meta = metadata.unwrap();
    assert_eq!(meta.session_id, Some("f5b6aaac-4316-454a".to_string()));
    assert_eq!(meta.model, Some("claude-opus-4-5-20251101".to_string()));
    assert_eq!(meta.tools.len(), 2);
    assert_eq!(meta.tools[0].name, "Glob");
    assert_eq!(meta.tools[1].name, "Read");
}

#[test]
fn test_extract_metadata_from_events_no_system_event() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Hello!".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_none());
}

#[test]
fn test_extract_metadata_from_events_non_init_system_event() {
    let events = vec![StreamEvent::System(SystemEvent {
        subtype: Some("other".to_string()),
        session_id: Some("abc".to_string()),
        model: Some("claude".to_string()),
        tools: vec![],
    })];

    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_none());
}

#[test]
fn test_extract_metadata_from_events_empty_events() {
    let events: Vec<StreamEvent> = vec![];
    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_none());
}

#[test]
fn test_extract_metadata_from_events_first_init_wins() {
    // If there are multiple init events, the first one should be returned
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("first-session".to_string()),
            model: Some("model-1".to_string()),
            tools: vec![],
        }),
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("second-session".to_string()),
            model: Some("model-2".to_string()),
            tools: vec![],
        }),
    ];

    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_some());
    let meta = metadata.unwrap();
    assert_eq!(meta.session_id, Some("first-session".to_string()));
    assert_eq!(meta.model, Some("model-1".to_string()));
}

#[test]
fn test_extract_metadata_from_events_or_default_with_init() {
    let events = vec![StreamEvent::System(SystemEvent {
        subtype: Some("init".to_string()),
        session_id: Some("session-456".to_string()),
        model: Some("claude-sonnet".to_string()),
        tools: vec![Tool {
            name: "Write".to_string(),
            description: None,
        }],
    })];

    let metadata = extract_metadata_from_events_or_default(&events);
    assert!(!metadata.is_empty());
    assert_eq!(metadata.session_id, Some("session-456".to_string()));
    assert_eq!(metadata.model, Some("claude-sonnet".to_string()));
    assert_eq!(metadata.tools.len(), 1);
}

#[test]
fn test_extract_metadata_from_events_or_default_no_init() {
    let events: Vec<StreamEvent> = vec![];
    let metadata = extract_metadata_from_events_or_default(&events);
    assert!(metadata.is_empty());
    assert_eq!(metadata.session_id, None);
    assert_eq!(metadata.model, None);
    assert!(metadata.tools.is_empty());
}

#[test]
fn test_iteration_metadata_serialization() {
    let metadata = IterationMetadata {
        session_id: Some("abc-123".to_string()),
        model: Some("claude-opus-4-5".to_string()),
        tools: vec![
            Tool {
                name: "Read".to_string(),
                description: Some("Read files".to_string()),
            },
            Tool {
                name: "Edit".to_string(),
                description: None,
            },
        ],
    };

    let json = serde_json::to_string(&metadata).unwrap();
    let roundtrip: IterationMetadata = serde_json::from_str(&json).unwrap();

    assert_eq!(metadata, roundtrip);
}

#[test]
fn test_iteration_metadata_serialization_empty_fields_skipped() {
    let metadata = IterationMetadata {
        session_id: Some("abc".to_string()),
        model: None,
        tools: vec![],
    };

    let json = serde_json::to_string(&metadata).unwrap();
    // Empty model and tools should be skipped
    assert!(!json.contains("\"model\""));
    assert!(!json.contains("\"tools\""));
    // But session_id should be present
    assert!(json.contains("\"session_id\""));
}

#[test]
fn test_extract_metadata_init_at_end() {
    // Init event at the end of the list (should still be found)
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Result(ResultEvent {
            subtype: None,
            total_cost_usd: Some(0.01),
            cost_usd: None,
            duration_ms: None,
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        }),
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("late-init".to_string()),
            model: Some("claude-late".to_string()),
            tools: vec![],
        }),
    ];

    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_some());
    let meta = metadata.unwrap();
    assert_eq!(meta.session_id, Some("late-init".to_string()));
}

#[test]
fn test_extract_metadata_partial_fields() {
    // Only some fields present - others should be None/empty
    let events = vec![StreamEvent::System(SystemEvent {
        subtype: Some("init".to_string()),
        session_id: Some("session-only".to_string()),
        model: None,
        tools: vec![],
    })];

    let metadata = extract_metadata_from_events(&events);
    assert!(metadata.is_some());
    let meta = metadata.unwrap();
    assert_eq!(meta.session_id, Some("session-only".to_string()));
    assert_eq!(meta.model, None);
    assert!(meta.tools.is_empty());
    assert!(!meta.is_empty()); // Still not empty since session_id is set
}
