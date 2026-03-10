use chrono::Utc;

use super::types::EditMessage;
use super::*;
use crate::iteration::IterationLog;

fn make_log(sequence: u32, prompt: Option<&str>, response: Option<&str>) -> IterationLog {
    IterationLog {
        sequence,
        started_at: Utc::now(),
        completed_at: Utc::now(),
        exit_code: 0,
        pending_before: 0,
        pending_after: 0,
        prompt: prompt.map(String::from),
        response: response.map(String::from),
        metadata: None,
        tool_calls: vec![],
        chunks: vec![],
        output_blocks: vec![],
        strategy_name: None,
        persona: None,
    }
}

#[test]
fn test_iterations_to_messages_basic() {
    let logs = vec![
        make_log(1, Some("Hello"), Some("Hi there")),
        make_log(2, Some("How are you?"), Some("I'm good")),
    ];

    let messages = iterations_to_messages(&logs);
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "Hello");
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content, "Hi there");
    assert_eq!(messages[2].role, "user");
    assert_eq!(messages[2].content, "How are you?");
    assert_eq!(messages[3].role, "assistant");
    assert_eq!(messages[3].content, "I'm good");
}

#[test]
fn test_iterations_to_messages_response_only() {
    let logs = vec![
        make_log(1, None, Some("response without prompt")),
        make_log(2, Some("valid prompt"), Some("valid response")),
    ];

    let messages = iterations_to_messages(&logs);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "assistant");
    assert_eq!(messages[0].content, "response without prompt");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[1].content, "valid prompt");
    assert_eq!(messages[2].role, "assistant");
    assert_eq!(messages[2].content, "valid response");
}

#[test]
fn test_iterations_to_messages_skips_neither() {
    let logs = vec![
        make_log(1, None, None),
        make_log(2, Some("prompt"), Some("response")),
    ];

    let messages = iterations_to_messages(&logs);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "prompt");
}

#[test]
fn test_iterations_to_messages_handles_missing_response() {
    let logs = vec![make_log(1, Some("question"), None)];

    let messages = iterations_to_messages(&logs);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "question");
}

#[test]
fn test_iterations_to_messages_empty() {
    let messages = iterations_to_messages(&[]);
    assert!(messages.is_empty());
}

#[test]
fn test_messages_to_edit_toml_basic() {
    let msgs = vec![
        EditMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        },
        EditMessage {
            role: "assistant".to_string(),
            content: "Hi there".to_string(),
        },
    ];

    let toml = messages_to_edit_toml(&msgs, "test-session");

    assert!(toml.contains("[[messages]]"));
    assert!(toml.contains("role = \"user\""));
    assert!(toml.contains("role = \"assistant\""));
    assert!(toml.contains("Hello"));
    assert!(toml.contains("Hi there"));
}

#[test]
fn test_messages_to_edit_toml_includes_header() {
    let toml = messages_to_edit_toml(&[], "my-session");

    assert!(toml.contains("# Session: my-session"));
    assert!(toml.contains("# Editing conversation history"));
    assert!(toml.contains("# Roles: \"user\" or \"assistant\""));
}

#[test]
fn test_messages_to_edit_toml_escapes_triple_quotes() {
    let msgs = vec![EditMessage {
        role: "assistant".to_string(),
        content: "Here is some code:\n```python\nx = \"\"\"hello\"\"\"\n```".to_string(),
    }];

    let toml_str = messages_to_edit_toml(&msgs, "test-session");

    // The generated TOML must be parseable.
    let parsed: toml::Value = toml::from_str(&toml_str)
        .unwrap_or_else(|e| panic!("Generated TOML is not valid: {e}\n---\n{toml_str}"));

    // Verify the content round-trips correctly.
    let messages = parsed
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("should have messages array");
    assert_eq!(messages.len(), 1);
    let content = messages[0]
        .get("content")
        .and_then(|v| v.as_str())
        .expect("should have content");
    // TOML multi-line basic strings include the trailing newline before the
    // closing `"""`, so the parsed content has a trailing `\n`.
    assert_eq!(content, format!("{}\n", msgs[0].content));
}

#[test]
fn test_messages_to_edit_toml_empty() {
    let toml = messages_to_edit_toml(&[], "empty-session");

    // Should contain header but no actual message blocks
    assert!(toml.contains("# Session: empty-session"));
    // The only [[messages]] references should be in comments
    for line in toml.lines() {
        if line.contains("[[messages]]") {
            assert!(line.starts_with('#'), "Found non-comment [[messages]] line");
        }
    }
    // Should not contain any role assignments
    assert!(!toml.contains("role = "));
}

// =========================================================================
// parse_edit_toml tests
// =========================================================================

#[test]
fn test_parse_edit_toml_valid() {
    let input = r#"
[[messages]]
role = "user"
content = """
Hello world
"""

[[messages]]
role = "assistant"
content = """
Hi there
"""
"#;
    let msgs = parse_edit_toml(input).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[0].content, "Hello world");
    assert_eq!(msgs[1].role, "assistant");
    assert_eq!(msgs[1].content, "Hi there");
}

#[test]
fn test_parse_edit_toml_invalid_syntax() {
    let input = "this is not valid TOML [[[";
    let result = parse_edit_toml(input);
    assert!(result.is_err());
    match result.unwrap_err() {
        EditError::ParseEditToml(_) => {}
        e => panic!("Expected ParseEditToml, got: {e}"),
    }
}

#[test]
fn test_parse_edit_toml_invalid_role() {
    let input = r#"
[[messages]]
role = "system"
content = """
Hello
"""
"#;
    let result = parse_edit_toml(input);
    assert!(result.is_err());
    match result.unwrap_err() {
        EditError::InvalidRole { role, index } => {
            assert_eq!(role, "system");
            assert_eq!(index, 0);
        }
        e => panic!("Expected InvalidRole, got: {e}"),
    }
}

#[test]
fn test_parse_edit_toml_missing_role() {
    let input = r#"
[[messages]]
content = """
Hello
"""
"#;
    let result = parse_edit_toml(input);
    assert!(result.is_err());
    match result.unwrap_err() {
        EditError::ParseEditToml(msg) => {
            assert!(msg.contains("role"));
        }
        e => panic!("Expected ParseEditToml, got: {e}"),
    }
}

#[test]
fn test_parse_edit_toml_missing_content() {
    let input = r#"
[[messages]]
role = "user"
"#;
    let result = parse_edit_toml(input);
    assert!(result.is_err());
    match result.unwrap_err() {
        EditError::ParseEditToml(msg) => {
            assert!(msg.contains("content"));
        }
        e => panic!("Expected ParseEditToml, got: {e}"),
    }
}

#[test]
fn test_parse_edit_toml_content_trimming() {
    let input = r#"
[[messages]]
role = "user"
content = """
Hello
"""
"#;
    let msgs = parse_edit_toml(input).unwrap();
    // The trailing newline from TOML multi-line string should be trimmed
    assert_eq!(msgs[0].content, "Hello");
}

#[test]
fn test_parse_edit_toml_empty_messages() {
    let input = "# Just a comment, no messages\n";
    let msgs = parse_edit_toml(input).unwrap();
    assert!(msgs.is_empty());
}

#[test]
fn test_parse_edit_toml_roundtrip() {
    let original = vec![
        EditMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        },
        EditMessage {
            role: "assistant".to_string(),
            content: "Hi there".to_string(),
        },
    ];

    let toml_str = messages_to_edit_toml(&original, "test");
    let parsed = parse_edit_toml(&toml_str).unwrap();

    assert_eq!(parsed, original);
}

// =========================================================================
// pair_messages_to_iterations tests
// =========================================================================

#[test]
fn test_pair_messages_user_assistant() {
    let msgs = vec![
        EditMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
        },
        EditMessage {
            role: "assistant".to_string(),
            content: "Hi".to_string(),
        },
    ];

    let pairs = pair_messages_to_iterations(&msgs);
    assert_eq!(pairs.len(), 1);
    assert_eq!(
        pairs[0],
        (Some("Hello".to_string()), Some("Hi".to_string()))
    );
}

#[test]
fn test_pair_messages_user_only() {
    let msgs = vec![EditMessage {
        role: "user".to_string(),
        content: "Hello".to_string(),
    }];

    let pairs = pair_messages_to_iterations(&msgs);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0], (Some("Hello".to_string()), None));
}

#[test]
fn test_pair_messages_assistant_only() {
    let msgs = vec![EditMessage {
        role: "assistant".to_string(),
        content: "Hi".to_string(),
    }];

    let pairs = pair_messages_to_iterations(&msgs);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0], (None, Some("Hi".to_string())));
}

#[test]
fn test_pair_messages_multiple_pairs() {
    let msgs = vec![
        EditMessage {
            role: "user".to_string(),
            content: "Q1".to_string(),
        },
        EditMessage {
            role: "assistant".to_string(),
            content: "A1".to_string(),
        },
        EditMessage {
            role: "user".to_string(),
            content: "Q2".to_string(),
        },
        EditMessage {
            role: "assistant".to_string(),
            content: "A2".to_string(),
        },
    ];

    let pairs = pair_messages_to_iterations(&msgs);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], (Some("Q1".to_string()), Some("A1".to_string())));
    assert_eq!(pairs[1], (Some("Q2".to_string()), Some("A2".to_string())));
}

#[test]
fn test_pair_messages_empty() {
    let pairs = pair_messages_to_iterations(&[]);
    assert!(pairs.is_empty());
}

#[test]
fn test_pair_messages_consecutive_users() {
    let msgs = vec![
        EditMessage {
            role: "user".to_string(),
            content: "Q1".to_string(),
        },
        EditMessage {
            role: "user".to_string(),
            content: "Q2".to_string(),
        },
        EditMessage {
            role: "assistant".to_string(),
            content: "A2".to_string(),
        },
    ];

    let pairs = pair_messages_to_iterations(&msgs);
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], (Some("Q1".to_string()), None));
    assert_eq!(pairs[1], (Some("Q2".to_string()), Some("A2".to_string())));
}

// =========================================================================
// plan_iteration_updates tests
// =========================================================================

#[test]
fn test_plan_iteration_updates_rewrite() {
    let logs = vec![make_log(1, Some("Hello"), Some("Hi"))];
    let new_iters = vec![(Some("Hello modified".to_string()), Some("Hi".to_string()))];

    let plan = plan_iteration_updates(&logs, &new_iters);
    assert_eq!(plan.len(), 1);
    assert_eq!(
        plan[0],
        IterationUpdate::Rewrite {
            sequence: 1,
            prompt: Some("Hello modified".to_string()),
            response: Some("Hi".to_string()),
        }
    );
}

#[test]
fn test_plan_iteration_updates_no_changes() {
    let logs = vec![make_log(1, Some("Hello"), Some("Hi"))];
    let new_iters = vec![(Some("Hello".to_string()), Some("Hi".to_string()))];

    let plan = plan_iteration_updates(&logs, &new_iters);
    assert!(plan.is_empty());
}

#[test]
fn test_plan_iteration_updates_delete() {
    let logs = vec![
        make_log(1, Some("Hello"), Some("Hi")),
        make_log(2, Some("Bye"), Some("See ya")),
    ];
    let new_iters = vec![(Some("Hello".to_string()), Some("Hi".to_string()))];

    let plan = plan_iteration_updates(&logs, &new_iters);
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0], IterationUpdate::Delete { sequence: 2 });
}

#[test]
fn test_plan_iteration_updates_create() {
    let logs = vec![make_log(1, Some("Hello"), Some("Hi"))];
    let new_iters = vec![
        (Some("Hello".to_string()), Some("Hi".to_string())),
        (Some("New Q".to_string()), Some("New A".to_string())),
    ];

    let plan = plan_iteration_updates(&logs, &new_iters);
    assert_eq!(plan.len(), 1);
    assert_eq!(
        plan[0],
        IterationUpdate::Create {
            sequence: 2,
            prompt: Some("New Q".to_string()),
            response: Some("New A".to_string()),
        }
    );
}

#[test]
fn test_plan_iteration_updates_mixed() {
    let logs = vec![
        make_log(1, Some("A"), Some("B")),
        make_log(2, Some("C"), Some("D")),
        make_log(3, Some("E"), Some("F")),
    ];
    // Same count but positions 1 and 3 changed
    let new_iters = vec![
        (Some("A-mod".to_string()), Some("B".to_string())), // edit at 1
        (Some("C".to_string()), Some("D".to_string())),     // no change at 2
        (Some("G".to_string()), Some("H".to_string())),     // edit at 3
    ];

    let plan = plan_iteration_updates(&logs, &new_iters);
    assert_eq!(plan.len(), 2);
    assert_eq!(
        plan[0],
        IterationUpdate::Rewrite {
            sequence: 1,
            prompt: Some("A-mod".to_string()),
            response: Some("B".to_string()),
        }
    );
    assert_eq!(
        plan[1],
        IterationUpdate::Rewrite {
            sequence: 3,
            prompt: Some("G".to_string()),
            response: Some("H".to_string()),
        }
    );
}

#[test]
fn test_plan_iteration_updates_delete_and_create() {
    let logs = vec![
        make_log(1, Some("A"), Some("B")),
        make_log(2, Some("C"), Some("D")),
    ];
    // Delete iteration 2, add a new one with different content
    let new_iters = vec![
        (Some("A".to_string()), Some("B".to_string())), // unchanged
        (Some("X".to_string()), Some("Y".to_string())), // replaces C/D
        (Some("New".to_string()), Some("Added".to_string())), // new
    ];

    let plan = plan_iteration_updates(&logs, &new_iters);
    assert_eq!(plan.len(), 2);
    // Rewrite seq 2 (C/D -> X/Y)
    assert_eq!(
        plan[0],
        IterationUpdate::Rewrite {
            sequence: 2,
            prompt: Some("X".to_string()),
            response: Some("Y".to_string()),
        }
    );
    // Create seq 3
    assert_eq!(
        plan[1],
        IterationUpdate::Create {
            sequence: 3,
            prompt: Some("New".to_string()),
            response: Some("Added".to_string()),
        }
    );
}

// =========================================================================
// TOML trailing quote edge case tests
// =========================================================================

fn assert_toml_roundtrip(content: &str) {
    let original = vec![EditMessage {
        role: "user".to_string(),
        content: content.to_string(),
    }];
    let toml_str = messages_to_edit_toml(&original, "test");
    let parsed = parse_edit_toml(&toml_str).unwrap_or_else(|e| {
        panic!(
            "Failed to round-trip content {:?}: {e}\n---\n{toml_str}",
            content
        )
    });
    assert_eq!(
        parsed, original,
        "Round-trip failed for content: {content:?}"
    );
}

#[test]
fn test_toml_roundtrip_trailing_single_quote() {
    assert_toml_roundtrip("ends with a quote\"");
}

#[test]
fn test_toml_roundtrip_trailing_double_quote() {
    assert_toml_roundtrip("ends with two quotes\"\"");
}

#[test]
fn test_toml_roundtrip_four_consecutive_quotes() {
    assert_toml_roundtrip("has \"\"\"\" four quotes");
}

#[test]
fn test_toml_roundtrip_five_consecutive_quotes() {
    assert_toml_roundtrip("has \"\"\"\"\" five quotes");
}
