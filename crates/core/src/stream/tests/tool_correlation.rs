use crate::stream::*;

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
fn test_tool_correlator_basic() {
    let mut correlator = ToolCorrelator::new();

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
