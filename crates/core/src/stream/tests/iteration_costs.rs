use crate::stream::*;

#[test]
fn test_iteration_costs_new_is_empty() {
    let costs = IterationCosts::new();
    assert!(costs.is_empty());
    assert_eq!(costs.cost_usd, None);
    assert_eq!(costs.duration_ms, None);
    assert!(costs.usage.is_none());
}

#[test]
fn test_iteration_costs_is_empty_with_cost() {
    let costs = IterationCosts {
        cost_usd: Some(0.05),
        duration_ms: None,
        usage: None,
    };
    assert!(!costs.is_empty());
}

#[test]
fn test_iteration_costs_is_empty_with_duration() {
    let costs = IterationCosts {
        cost_usd: None,
        duration_ms: Some(5000),
        usage: None,
    };
    assert!(!costs.is_empty());
}

#[test]
fn test_iteration_costs_is_empty_with_usage() {
    let costs = IterationCosts {
        cost_usd: None,
        duration_ms: None,
        usage: Some(Usage::default()),
    };
    assert!(!costs.is_empty());
}

#[test]
fn test_result_event_extract_costs_full() {
    let event = ResultEvent {
        subtype: Some("success".to_string()),
        total_cost_usd: Some(0.226354),
        cost_usd: None,
        duration_ms: Some(40966),
        duration_api_ms: Some(35000),
        usage: Some(Usage {
            input_tokens: 712,
            output_tokens: 2971,
            cache_read_input_tokens: Some(107476),
            cache_creation_input_tokens: Some(12504),
        }),
        session_id: Some("session-123".to_string()),
        num_turns: Some(3),
        result: None,
    };

    let costs = event.extract_costs();
    assert_eq!(costs.cost_usd, Some(0.226354));
    assert_eq!(costs.duration_ms, Some(40966));
    assert!(costs.usage.is_some());
    let usage = costs.usage.unwrap();
    assert_eq!(usage.input_tokens, 712);
    assert_eq!(usage.output_tokens, 2971);
    assert_eq!(usage.cache_read_input_tokens, Some(107476));
    assert_eq!(usage.cache_creation_input_tokens, Some(12504));
}

#[test]
fn test_result_event_extract_costs_uses_alternative_cost_field() {
    // When total_cost_usd is None but cost_usd is set
    let event = ResultEvent {
        subtype: None,
        total_cost_usd: None,
        cost_usd: Some(0.123),
        duration_ms: Some(1000),
        duration_api_ms: None,
        usage: None,
        session_id: None,
        num_turns: None,
        result: None,
    };

    let costs = event.extract_costs();
    assert_eq!(costs.cost_usd, Some(0.123));
    assert_eq!(costs.duration_ms, Some(1000));
    assert!(costs.usage.is_none());
}

#[test]
fn test_result_event_extract_costs_prefers_total_cost_usd() {
    // When both fields are set, total_cost_usd takes precedence
    let event = ResultEvent {
        subtype: None,
        total_cost_usd: Some(0.50),
        cost_usd: Some(0.25),
        duration_ms: None,
        duration_api_ms: None,
        usage: None,
        session_id: None,
        num_turns: None,
        result: None,
    };

    let costs = event.extract_costs();
    assert_eq!(costs.cost_usd, Some(0.50)); // total_cost_usd wins
}

#[test]
fn test_result_event_extract_costs_empty() {
    let event = ResultEvent {
        subtype: None,
        total_cost_usd: None,
        cost_usd: None,
        duration_ms: None,
        duration_api_ms: None,
        usage: None,
        session_id: None,
        num_turns: None,
        result: None,
    };

    let costs = event.extract_costs();
    assert!(costs.is_empty());
}

#[test]
fn test_extract_costs_from_events_with_result() {
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
            subtype: Some("success".to_string()),
            total_cost_usd: Some(0.05),
            cost_usd: None,
            duration_ms: Some(5000),
            duration_api_ms: None,
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: 200,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            }),
            session_id: None,
            num_turns: None,
            result: None,
        }),
    ];

    let costs = extract_costs_from_events(&events);
    assert!(costs.is_some());
    let c = costs.unwrap();
    assert_eq!(c.cost_usd, Some(0.05));
    assert_eq!(c.duration_ms, Some(5000));
    assert!(c.usage.is_some());
}

#[test]
fn test_extract_costs_from_events_no_result() {
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("session-123".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            tools: vec![],
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

    let costs = extract_costs_from_events(&events);
    assert!(costs.is_none());
}

#[test]
fn test_extract_costs_from_events_empty_slice() {
    let events: Vec<StreamEvent> = vec![];
    let costs = extract_costs_from_events(&events);
    assert!(costs.is_none());
}

#[test]
fn test_extract_costs_from_events_multiple_results_takes_first() {
    let events = vec![
        StreamEvent::Result(ResultEvent {
            subtype: None,
            total_cost_usd: Some(0.10),
            cost_usd: None,
            duration_ms: Some(1000),
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        }),
        StreamEvent::Result(ResultEvent {
            subtype: None,
            total_cost_usd: Some(0.20),
            cost_usd: None,
            duration_ms: Some(2000),
            duration_api_ms: None,
            usage: None,
            session_id: None,
            num_turns: None,
            result: None,
        }),
    ];

    let costs = extract_costs_from_events(&events);
    assert!(costs.is_some());
    let c = costs.unwrap();
    assert_eq!(c.cost_usd, Some(0.10)); // First result wins
    assert_eq!(c.duration_ms, Some(1000));
}

#[test]
fn test_extract_costs_from_events_or_default_with_result() {
    let events = vec![StreamEvent::Result(ResultEvent {
        subtype: None,
        total_cost_usd: Some(0.15),
        cost_usd: None,
        duration_ms: Some(3000),
        duration_api_ms: None,
        usage: None,
        session_id: None,
        num_turns: None,
        result: None,
    })];

    let costs = extract_costs_from_events_or_default(&events);
    assert!(!costs.is_empty());
    assert_eq!(costs.cost_usd, Some(0.15));
    assert_eq!(costs.duration_ms, Some(3000));
}

#[test]
fn test_extract_costs_from_events_or_default_no_result() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text {
                text: "Hello!".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let costs = extract_costs_from_events_or_default(&events);
    assert!(costs.is_empty());
    assert_eq!(costs.cost_usd, None);
    assert_eq!(costs.duration_ms, None);
    assert!(costs.usage.is_none());
}

#[test]
fn test_iteration_costs_serialization_round_trip() {
    let costs = IterationCosts {
        cost_usd: Some(0.226354),
        duration_ms: Some(40966),
        usage: Some(Usage {
            input_tokens: 712,
            output_tokens: 2971,
            cache_read_input_tokens: Some(107476),
            cache_creation_input_tokens: Some(12504),
        }),
    };

    let json = serde_json::to_string(&costs).unwrap();
    let parsed: IterationCosts = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, costs);
}

#[test]
fn test_iteration_costs_empty_fields_skip_serialization() {
    let costs = IterationCosts {
        cost_usd: Some(0.05),
        duration_ms: None,
        usage: None,
    };

    let json = serde_json::to_string(&costs).unwrap();
    // Should not contain duration_ms or usage fields
    assert!(!json.contains("duration_ms"));
    assert!(!json.contains("usage"));
    assert!(json.contains("cost_usd"));
}

#[test]
fn test_extract_costs_result_at_end() {
    // Result event typically comes at the end of the stream
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
            session_id: Some("session-123".to_string()),
            model: Some("claude-opus-4-5".to_string()),
            tools: vec![],
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "Implementing feature...".to_string(),
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
                    content: Some("File updated".to_string()),
                    is_error: false,
                }],
            },
        }),
        StreamEvent::Result(ResultEvent {
            subtype: Some("success".to_string()),
            total_cost_usd: Some(0.30),
            cost_usd: None,
            duration_ms: Some(60000),
            duration_api_ms: None,
            usage: Some(Usage {
                input_tokens: 1000,
                output_tokens: 5000,
                cache_read_input_tokens: Some(50000),
                cache_creation_input_tokens: None,
            }),
            session_id: None,
            num_turns: Some(5),
            result: None,
        }),
    ];

    let costs = extract_costs_from_events(&events);
    assert!(costs.is_some());
    let c = costs.unwrap();
    assert_eq!(c.cost_usd, Some(0.30));
    assert_eq!(c.duration_ms, Some(60000));
    let usage = c.usage.unwrap();
    assert_eq!(usage.input_tokens, 1000);
    assert_eq!(usage.output_tokens, 5000);
    assert_eq!(usage.cache_read_input_tokens, Some(50000));
    assert_eq!(usage.cache_creation_input_tokens, None);
}
