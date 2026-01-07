use crate::stream::*;

// Tests for text accumulation across streaming events (Story #15)

#[test]
fn test_text_accumulator_new() {
    let accumulator = TextAccumulator::new();
    assert_eq!(accumulator.get_text(), "");
    assert!(accumulator.completed_messages().is_empty());
}

#[test]
fn test_text_accumulator_single_event() {
    let mut accumulator = TextAccumulator::new();

    let event = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Hello, world!".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    let completed = accumulator.process_event(&event);
    assert!(completed.is_none()); // No message completed yet
    assert_eq!(accumulator.get_text(), "Hello, world!");
}

#[test]
fn test_text_accumulator_multiple_events_same_message_id() {
    let mut accumulator = TextAccumulator::new();

    let event1 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Hello, ".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    let event2 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "world!".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event1);
    let completed = accumulator.process_event(&event2);

    assert!(completed.is_none()); // Same message ID, not completed yet
    assert_eq!(accumulator.get_text(), "Hello, world!");
}

#[test]
fn test_text_accumulator_new_message_id_completes_previous() {
    let mut accumulator = TextAccumulator::new();

    let event1 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "First message.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    let event2 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_02".to_string()),
            content: vec![ContentBlock::Text {
                text: "Second message.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event1);
    let completed = accumulator.process_event(&event2);

    // First message should be completed when we see msg_02
    assert!(completed.is_some());
    let msg = completed.unwrap();
    assert_eq!(msg.id, Some("msg_01".to_string()));
    assert_eq!(msg.text, "First message.");

    // Current buffer should have second message
    assert_eq!(accumulator.get_text(), "Second message.");
}

#[test]
fn test_text_accumulator_finish() {
    let mut accumulator = TextAccumulator::new();

    let event = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Final message.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event);
    let completed = accumulator.finish();

    assert!(completed.is_some());
    let msg = completed.unwrap();
    assert_eq!(msg.id, Some("msg_01".to_string()));
    assert_eq!(msg.text, "Final message.");

    // Buffer should be empty after finish
    assert_eq!(accumulator.get_text(), "");
}

#[test]
fn test_text_accumulator_finish_empty() {
    let mut accumulator = TextAccumulator::new();
    let completed = accumulator.finish();
    assert!(completed.is_none());
}

#[test]
fn test_text_accumulator_completed_messages() {
    let mut accumulator = TextAccumulator::new();

    let event1 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "First.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    let event2 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_02".to_string()),
            content: vec![ContentBlock::Text {
                text: "Second.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event1);
    accumulator.process_event(&event2);
    accumulator.finish();

    let completed = accumulator.completed_messages();
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[0].text, "First.");
    assert_eq!(completed[1].text, "Second.");
}

#[test]
fn test_text_accumulator_get_all_text() {
    let mut accumulator = TextAccumulator::new();

    let event1 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "First. ".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    let event2 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_02".to_string()),
            content: vec![ContentBlock::Text {
                text: "Second.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event1);
    accumulator.process_event(&event2);

    // Before finish: first message is completed, second is in buffer
    assert_eq!(accumulator.get_all_text(), "First. Second.");

    accumulator.finish();

    // After finish: both messages are completed
    assert_eq!(accumulator.get_all_text(), "First. Second.");
}

#[test]
fn test_text_accumulator_ignores_non_assistant_events() {
    let mut accumulator = TextAccumulator::new();

    let system_event = StreamEvent::System(SystemEvent {
        subtype: Some("init".to_string()),
        session_id: Some("abc".to_string()),
        model: None,
        tools: vec![],
    });

    let user_event = StreamEvent::User(UserEvent {
        message: UserMessage {
            id: None,
            content: vec![ToolResult {
                result_type: Some("tool_result".to_string()),
                tool_use_id: Some("toolu_01".to_string()),
                content: Some("result".to_string()),
                is_error: false,
            }],
        },
    });

    let result_event = StreamEvent::Result(ResultEvent {
        subtype: None,
        total_cost_usd: Some(0.01),
        cost_usd: None,
        duration_ms: None,
        duration_api_ms: None,
        usage: None,
        session_id: None,
        num_turns: None,
        result: None,
    });

    assert!(accumulator.process_event(&system_event).is_none());
    assert!(accumulator.process_event(&user_event).is_none());
    assert!(accumulator.process_event(&result_event).is_none());

    assert_eq!(accumulator.get_text(), "");
}

#[test]
fn test_text_accumulator_reset() {
    let mut accumulator = TextAccumulator::new();

    let event = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Some text.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event);
    accumulator.finish();

    assert!(!accumulator.completed_messages().is_empty());

    accumulator.reset();

    assert_eq!(accumulator.get_text(), "");
    assert!(accumulator.completed_messages().is_empty());
}

#[test]
fn test_text_accumulator_with_none_message_ids() {
    let mut accumulator = TextAccumulator::new();

    // First event with no message ID
    let event1 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text {
                text: "First.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    // Second event with no message ID (same as first)
    let event2 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text {
                text: "Second.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event1);
    let completed = accumulator.process_event(&event2);

    // Same message ID (None), so should combine
    assert!(completed.is_none());
    assert_eq!(accumulator.get_text(), "First.Second.");
}

#[test]
fn test_text_accumulator_none_to_some_message_id() {
    let mut accumulator = TextAccumulator::new();

    let event1 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text {
                text: "First.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    let event2 = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Second.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event1);
    let completed = accumulator.process_event(&event2);

    // ID changed from None to Some, should complete first message
    assert!(completed.is_some());
    let msg = completed.unwrap();
    assert_eq!(msg.id, None);
    assert_eq!(msg.text, "First.");
}

#[test]
fn test_text_accumulator_tool_only_events() {
    let mut accumulator = TextAccumulator::new();

    let event = StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::ToolUse {
                id: "toolu_01".to_string(),
                name: "Read".to_string(),
                input: serde_json::json!({"file_path": "/test.rs"}),
            }],
            model: None,
            stop_reason: None,
        },
    });

    accumulator.process_event(&event);

    // Tool-only events add empty string to buffer
    assert_eq!(accumulator.get_text(), "");
}

#[test]
fn test_accumulate_text_function() {
    let events = [
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
                    text: "Hello, ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "world!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
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

    let text = accumulate_text(events.iter());
    assert_eq!(text, "Hello, world!");
}

#[test]
fn test_accumulate_text_multiple_messages() {
    let events = [
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First message. ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Second message.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    let text = accumulate_text(events.iter());
    assert_eq!(text, "First message. Second message.");
}

#[test]
fn test_accumulate_text_empty_events() {
    let events: Vec<StreamEvent> = vec![];
    let text = accumulate_text(events.iter());
    assert_eq!(text, "");
}

#[test]
fn test_accumulate_text_matches_extract_text_from_events() {
    // This test verifies that accumulate_text produces the same result
    // as extract_text_from_events (matching plain-text mode output)
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
                content: vec![
                    ContentBlock::Text {
                        text: "Let me search. ".to_string(),
                    },
                    ContentBlock::ToolUse {
                        id: "toolu_01".to_string(),
                        name: "Glob".to_string(),
                        input: serde_json::json!({}),
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
                    content: Some("/test.rs".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_02".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Found it!".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    let accumulated = accumulate_text(events.iter());
    let extracted = extract_text_from_events(&events);

    assert_eq!(accumulated, extracted);
}

#[test]
fn test_text_accumulator_realistic_streaming_scenario() {
    // Simulates a realistic Claude streaming scenario where text arrives
    // in chunks across multiple events with the same message ID
    let mut accumulator = TextAccumulator::new();

    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "I'll ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "help you ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "implement ".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "this feature.".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    for event in &events {
        accumulator.process_event(event);
    }

    assert_eq!(
        accumulator.get_text(),
        "I'll help you implement this feature."
    );

    // Finish and verify
    accumulator.finish();
    assert_eq!(accumulator.completed_messages().len(), 1);
    assert_eq!(
        accumulator.get_all_text(),
        "I'll help you implement this feature."
    );
}
