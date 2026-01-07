use crate::stream::*;

#[test]
fn test_tool_interaction_from_invocation() {
    let invocation = ToolInvocation {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({"file_path": "/src/main.rs"}),
    };

    let interaction = ToolInteraction::from_invocation(&invocation);
    assert_eq!(interaction.id, "toolu_01");
    assert_eq!(interaction.name, "Read");
    assert_eq!(
        interaction.input,
        serde_json::json!({"file_path": "/src/main.rs"})
    );
    assert!(interaction.result.is_none());
    assert!(!interaction.is_error);
}

#[test]
fn test_tool_interaction_from_invocation_and_result() {
    let invocation = ToolInvocation {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({"file_path": "/src/main.rs"}),
    };
    let result = ToolResult {
        result_type: Some("tool_result".to_string()),
        tool_use_id: Some("toolu_01".to_string()),
        content: Some("fn main() {}".to_string()),
        is_error: false,
    };

    let interaction = ToolInteraction::from_invocation_and_result(&invocation, &result);
    assert_eq!(interaction.id, "toolu_01");
    assert_eq!(interaction.name, "Read");
    assert_eq!(interaction.result, Some("fn main() {}".to_string()));
    assert!(!interaction.is_error);
}

#[test]
fn test_tool_interaction_from_invocation_and_error_result() {
    let invocation = ToolInvocation {
        id: "toolu_01".to_string(),
        name: "Edit".to_string(),
        input: serde_json::json!({"file_path": "/src/main.rs", "old_string": "foo", "new_string": "bar"}),
    };
    let result = ToolResult {
        result_type: Some("tool_result".to_string()),
        tool_use_id: Some("toolu_01".to_string()),
        content: Some("File not found".to_string()),
        is_error: true,
    };

    let interaction = ToolInteraction::from_invocation_and_result(&invocation, &result);
    assert_eq!(interaction.name, "Edit");
    assert_eq!(interaction.result, Some("File not found".to_string()));
    assert!(interaction.is_error);
}

#[test]
fn test_correlate_tool_interactions_single_call() {
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({"pattern": "*.rs"}),
                }],
                model: None,
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: Some("user_msg_01".to_string()),
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_01".to_string()),
                    content: Some("src/main.rs\nsrc/lib.rs".to_string()),
                    is_error: false,
                }],
            },
        }),
    ];

    let interactions = correlate_tool_interactions(&events);
    assert_eq!(interactions.len(), 1);
    assert_eq!(interactions[0].id, "toolu_01");
    assert_eq!(interactions[0].name, "Glob");
    assert_eq!(
        interactions[0].result,
        Some("src/main.rs\nsrc/lib.rs".to_string())
    );
    assert!(!interactions[0].is_error);
}

#[test]
fn test_correlate_tool_interactions_multiple_calls() {
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
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
                        input: serde_json::json!({"file_path": "/Cargo.toml"}),
                    },
                ],
                model: None,
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![
                    ToolResult {
                        result_type: Some("tool_result".to_string()),
                        tool_use_id: Some("toolu_01".to_string()),
                        content: Some("src/main.rs".to_string()),
                        is_error: false,
                    },
                    ToolResult {
                        result_type: Some("tool_result".to_string()),
                        tool_use_id: Some("toolu_02".to_string()),
                        content: Some("[package]\nname = \"test\"".to_string()),
                        is_error: false,
                    },
                ],
            },
        }),
    ];

    let interactions = correlate_tool_interactions(&events);
    assert_eq!(interactions.len(), 2);
    assert_eq!(interactions[0].name, "Glob");
    assert_eq!(interactions[0].result, Some("src/main.rs".to_string()));
    assert_eq!(interactions[1].name, "Read");
    assert_eq!(
        interactions[1].result,
        Some("[package]\nname = \"test\"".to_string())
    );
}

#[test]
fn test_correlate_tool_interactions_no_result() {
    // Tool call without matching result (stream interrupted)
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/src/main.rs"}),
            }],
            model: None,
            stop_reason: Some("tool_use".to_string()),
        },
    })];

    let interactions = correlate_tool_interactions(&events);
    assert_eq!(interactions.len(), 1);
    assert_eq!(interactions[0].id, "toolu_01");
    assert_eq!(interactions[0].name, "Read");
    assert!(interactions[0].result.is_none());
    assert!(!interactions[0].is_error);
}

#[test]
fn test_correlate_tool_interactions_empty_events() {
    let interactions = correlate_tool_interactions(&[]);
    assert!(interactions.is_empty());
}

#[test]
fn test_correlate_tool_interactions_no_tool_calls() {
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: None,
            model: None,
            tools: vec![],
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Hello, how can I help?".to_string(),
                }],
                model: None,
                stop_reason: Some("end_turn".to_string()),
            },
        }),
    ];

    let interactions = correlate_tool_interactions(&events);
    assert!(interactions.is_empty());
}

#[test]
fn test_correlate_tool_interactions_order_preserved() {
    // Tool calls across multiple assistant events
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({}),
                }],
                model: None,
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_01".to_string()),
                    content: Some("result1".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({}),
                }],
                model: None,
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_02".to_string()),
                    content: Some("result2".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_03".to_string()),
                content: vec![ContentBlock::ToolUse {
                    id: "toolu_03".to_string(),
                    name: "Edit".to_string(),
                    input: serde_json::json!({}),
                }],
                model: None,
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: None,
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_03".to_string()),
                    content: Some("result3".to_string()),
                    is_error: false,
                }],
            },
        }),
    ];

    let interactions = correlate_tool_interactions(&events);
    assert_eq!(interactions.len(), 3);
    assert_eq!(interactions[0].name, "Glob");
    assert_eq!(interactions[1].name, "Read");
    assert_eq!(interactions[2].name, "Edit");
}

#[test]
fn test_tool_correlator_basic() {
    let mut correlator = ToolCorrelator::new();
    assert_eq!(correlator.pending_count(), 0);
    assert!(correlator.completed_interactions().is_empty());

    // Process assistant event with tool call
    let assistant = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Glob".to_string(),
                input: serde_json::json!({"pattern": "*.rs"}),
            }],
            model: None,
            stop_reason: Some("tool_use".to_string()),
        },
    });

    let completed = correlator.process_event(&assistant);
    assert!(completed.is_empty());
    assert_eq!(correlator.pending_count(), 1);
    assert!(correlator.completed_interactions().is_empty());

    // Process user event with result
    let user = StreamEvent::User(UserEvent {
        message: UserMessage {
            id: None,
            content: vec![ToolResult {
                result_type: Some("tool_result".to_string()),
                tool_use_id: Some("toolu_01".to_string()),
                content: Some("main.rs\nlib.rs".to_string()),
                is_error: false,
            }],
        },
    });

    let completed = correlator.process_event(&user);
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].name, "Glob");
    assert_eq!(completed[0].result, Some("main.rs\nlib.rs".to_string()));
    assert_eq!(correlator.pending_count(), 0);
    assert_eq!(correlator.completed_interactions().len(), 1);
}

#[test]
fn test_tool_correlator_finish_with_pending() {
    let mut correlator = ToolCorrelator::new();

    // Add tool call without result
    let assistant = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({}),
            }],
            model: None,
            stop_reason: Some("tool_use".to_string()),
        },
    });
    correlator.process_event(&assistant);

    // Finish without providing result
    let interactions = correlator.finish();
    assert_eq!(interactions.len(), 1);
    assert_eq!(interactions[0].name, "Read");
    assert!(interactions[0].result.is_none());
}

#[test]
fn test_tool_correlator_multiple_concurrent_calls() {
    let mut correlator = ToolCorrelator::new();

    // Assistant calls two tools at once
    let assistant = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![
                ContentBlock::ToolUse {
                    id: "toolu_01".to_string(),
                    name: "Glob".to_string(),
                    input: serde_json::json!({}),
                },
                ContentBlock::ToolUse {
                    id: "toolu_02".to_string(),
                    name: "Read".to_string(),
                    input: serde_json::json!({}),
                },
            ],
            model: None,
            stop_reason: Some("tool_use".to_string()),
        },
    });
    correlator.process_event(&assistant);
    assert_eq!(correlator.pending_count(), 2);

    // Results come back for both
    let user = StreamEvent::User(UserEvent {
        message: UserMessage {
            id: None,
            content: vec![
                ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_01".to_string()),
                    content: Some("glob result".to_string()),
                    is_error: false,
                },
                ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_02".to_string()),
                    content: Some("read result".to_string()),
                    is_error: false,
                },
            ],
        },
    });
    let completed = correlator.process_event(&user);
    assert_eq!(completed.len(), 2);
    assert_eq!(correlator.pending_count(), 0);
}

#[test]
fn test_tool_correlator_ignores_other_events() {
    let mut correlator = ToolCorrelator::new();

    let system = StreamEvent::System(SystemEvent {
        subtype: Some("init".to_string()),
        session_id: None,
        model: None,
        tools: vec![],
    });
    let completed = correlator.process_event(&system);
    assert!(completed.is_empty());
    assert_eq!(correlator.pending_count(), 0);

    let result = StreamEvent::Result(ResultEvent {
        subtype: Some("success".to_string()),
        total_cost_usd: Some(0.05),
        cost_usd: None,
        duration_ms: Some(5000),
        duration_api_ms: None,
        usage: None,
        session_id: None,
        num_turns: None,
        result: None,
    });
    let completed = correlator.process_event(&result);
    assert!(completed.is_empty());
    assert_eq!(correlator.pending_count(), 0);
}

#[test]
fn test_tool_correlator_reset() {
    let mut correlator = ToolCorrelator::new();

    let assistant = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({}),
            }],
            model: None,
            stop_reason: None,
        },
    });
    correlator.process_event(&assistant);
    assert_eq!(correlator.pending_count(), 1);

    correlator.reset();
    assert_eq!(correlator.pending_count(), 0);
    assert!(correlator.completed_interactions().is_empty());
}

#[test]
fn test_tool_interaction_serialization_round_trip() {
    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({"file_path": "/src/main.rs", "limit": 100}),
        result: Some("fn main() {}".to_string()),
        is_error: false,
    };

    let json = serde_json::to_string(&interaction).unwrap();
    let parsed: ToolInteraction = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, interaction);
}

#[test]
fn test_tool_interaction_serialization_no_result() {
    let interaction = ToolInteraction {
        id: "toolu_01".to_string(),
        name: "Read".to_string(),
        input: serde_json::json!({}),
        result: None,
        is_error: false,
    };

    let json = serde_json::to_string(&interaction).unwrap();
    // result field should be skipped
    assert!(!json.contains("result"));
    // is_error should always be present (defaults to false)
    assert!(json.contains("is_error"));
}

#[test]
fn test_correlate_realistic_stream() {
    // Simulate a realistic stream with system, assistant, user, and result events
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("session-123".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            tools: vec![
                Tool {
                    name: "Glob".to_string(),
                    description: None,
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
                content: vec![
                    ContentBlock::Text {
                        text: "I'll search for Rust files and read the main file.".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01YWLzHW2VBHQSz8VV1oCGSp".to_string(),
                        name: "Glob".to_string(),
                        input: serde_json::json!({"pattern": "**/*.rs"}),
                    },
                ],
                model: Some("claude-opus-4-5".to_string()),
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: Some("user_01".to_string()),
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_01YWLzHW2VBHQSz8VV1oCGSp".to_string()),
                    content: Some("src/main.rs\nsrc/lib.rs\nsrc/utils.rs".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![
                    ContentBlock::Text {
                        text: "Found 3 Rust files. Let me read main.rs.".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_02KKvyfhUNr2Bdu32AKbDzmX".to_string(),
                        name: "Read".to_string(),
                        input: serde_json::json!({"file_path": "src/main.rs"}),
                    },
                ],
                model: Some("claude-opus-4-5".to_string()),
                stop_reason: Some("tool_use".to_string()),
            },
        }),
        StreamEvent::User(UserEvent {
            message: UserMessage {
                id: Some("user_02".to_string()),
                content: vec![ToolResult {
                    result_type: Some("tool_result".to_string()),
                    tool_use_id: Some("toolu_02KKvyfhUNr2Bdu32AKbDzmX".to_string()),
                    content: Some("fn main() {\n    println!(\"Hello, world!\");\n}".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_03".to_string()),
                content: vec![ContentBlock::Text {
                    text: "The main file prints 'Hello, world!'.".to_string(),
                }],
                model: Some("claude-opus-4-5".to_string()),
                stop_reason: Some("end_turn".to_string()),
            },
        }),
        StreamEvent::Result(ResultEvent {
            subtype: Some("success".to_string()),
            total_cost_usd: Some(0.05),
            cost_usd: None,
            duration_ms: Some(5000),
            duration_api_ms: None,
            usage: Some(Usage {
                input_tokens: 500,
                output_tokens: 200,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            }),
            session_id: None,
            num_turns: Some(3),
            result: None,
        }),
    ];

    let interactions = correlate_tool_interactions(&events);
    assert_eq!(interactions.len(), 2);

    assert_eq!(interactions[0].id, "toolu_01YWLzHW2VBHQSz8VV1oCGSp");
    assert_eq!(interactions[0].name, "Glob");
    assert_eq!(
        interactions[0].input,
        serde_json::json!({"pattern": "**/*.rs"})
    );
    assert_eq!(
        interactions[0].result,
        Some("src/main.rs\nsrc/lib.rs\nsrc/utils.rs".to_string())
    );
    assert!(!interactions[0].is_error);

    assert_eq!(interactions[1].id, "toolu_02KKvyfhUNr2Bdu32AKbDzmX");
    assert_eq!(interactions[1].name, "Read");
    assert_eq!(
        interactions[1].input,
        serde_json::json!({"file_path": "src/main.rs"})
    );
    assert!(interactions[1].result.is_some());
    assert!(!interactions[1].is_error);
}
