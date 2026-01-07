use crate::stream::*;

#[test]
fn test_parse_stream_line_system_event() {
    let line =
        r#"{"type":"system","subtype":"init","session_id":"abc-123","model":"claude-opus-4-5"}"#;
    match parse_stream_line(line) {
        ParsedLine::Event(StreamEvent::System(sys)) => {
            assert_eq!(sys.session_id, Some("abc-123".to_string()));
            assert_eq!(sys.model, Some("claude-opus-4-5".to_string()));
        }
        other => panic!("Expected System event, got {:?}", other),
    }
}

#[test]
fn test_parse_stream_line_assistant_event() {
    let line =
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hello, world!"}]}}"#;
    match parse_stream_line(line) {
        ParsedLine::Event(StreamEvent::Assistant(ast)) => {
            assert_eq!(ast.message.content.len(), 1);
            match &ast.message.content[0] {
                ContentBlock::Text { text } => assert_eq!(text, "Hello, world!"),
                _ => panic!("Expected Text content"),
            }
        }
        other => panic!("Expected Assistant event, got {:?}", other),
    }
}

#[test]
fn test_parse_stream_line_result_event() {
    let line = r#"{"type":"result","total_cost_usd":0.15,"duration_ms":30000}"#;
    match parse_stream_line(line) {
        ParsedLine::Event(StreamEvent::Result(res)) => {
            assert_eq!(res.total_cost_usd, Some(0.15));
            assert_eq!(res.duration_ms, Some(30000));
        }
        other => panic!("Expected Result event, got {:?}", other),
    }
}

#[test]
fn test_parse_stream_line_empty() {
    assert!(matches!(parse_stream_line(""), ParsedLine::Empty));
    assert!(matches!(parse_stream_line("   "), ParsedLine::Empty));
    assert!(matches!(parse_stream_line("\t\n"), ParsedLine::Empty));
}

#[test]
fn test_parse_stream_line_malformed_json() {
    let line = "this is not json";
    match parse_stream_line(line) {
        ParsedLine::Error {
            line: original,
            error,
        } => {
            assert_eq!(original, "this is not json");
            assert!(!error.is_empty());
        }
        other => panic!("Expected Error, got {:?}", other),
    }
}

#[test]
fn test_parse_stream_line_partial_json() {
    let line = r#"{"type":"system""#;
    match parse_stream_line(line) {
        ParsedLine::Error { error, .. } => {
            assert!(!error.is_empty());
        }
        other => panic!("Expected Error, got {:?}", other),
    }
}

#[test]
fn test_parse_stream_line_unknown_type() {
    // Unknown type should result in parse error
    let line = r#"{"type":"unknown_event_type"}"#;
    assert!(matches!(parse_stream_line(line), ParsedLine::Error { .. }));
}

#[test]
fn test_parse_stream_line_with_whitespace() {
    let line = r#"  {"type":"system","session_id":"abc"}  "#;
    match parse_stream_line(line) {
        ParsedLine::Event(StreamEvent::System(sys)) => {
            assert_eq!(sys.session_id, Some("abc".to_string()));
        }
        other => panic!("Expected System event, got {:?}", other),
    }
}

#[test]
fn test_parse_stream_output_multiple_events() {
    let output = r#"{"type":"system","session_id":"abc"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"}]}}
{"type":"result","total_cost_usd":0.01}"#;

    let (events, errors) = parse_stream_output(output);
    assert_eq!(events.len(), 3);
    assert!(errors.is_empty());

    assert!(matches!(events[0], StreamEvent::System(_)));
    assert!(matches!(events[1], StreamEvent::Assistant(_)));
    assert!(matches!(events[2], StreamEvent::Result(_)));
}

#[test]
fn test_parse_stream_output_with_empty_lines() {
    let output = r#"{"type":"system","session_id":"abc"}

{"type":"result","total_cost_usd":0.01}
"#;

    let (events, errors) = parse_stream_output(output);
    assert_eq!(events.len(), 2);
    assert!(errors.is_empty());
}

#[test]
fn test_parse_stream_output_with_errors() {
    let output = r#"{"type":"system","session_id":"abc"}
not valid json
{"type":"result","total_cost_usd":0.01}"#;

    let (events, errors) = parse_stream_output(output);
    assert_eq!(events.len(), 2);
    assert_eq!(errors.len(), 1);

    let (line_num, original, _error) = &errors[0];
    assert_eq!(*line_num, 2);
    assert_eq!(original, "not valid json");
}

#[test]
fn test_parse_stream_output_empty() {
    let (events, errors) = parse_stream_output("");
    assert!(events.is_empty());
    assert!(errors.is_empty());
}

#[test]
fn test_parse_stream_output_only_empty_lines() {
    let output = "\n\n\n";
    let (events, errors) = parse_stream_output(output);
    assert!(events.is_empty());
    assert!(errors.is_empty());
}

#[test]
fn test_parse_stream_output_preserves_order() {
    let output = r#"{"type":"system","subtype":"init"}
{"type":"assistant","message":{"content":[{"type":"text","text":"First"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Second"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Third"}]}}"#;

    let (events, _) = parse_stream_output(output);
    assert_eq!(events.len(), 4);

    // Verify order: system, then three assistants
    assert!(matches!(events[0], StreamEvent::System(_)));

    for (i, expected_text) in [(1, "First"), (2, "Second"), (3, "Third")] {
        match &events[i] {
            StreamEvent::Assistant(ast) => match &ast.message.content[0] {
                ContentBlock::Text { text } => assert_eq!(text, expected_text),
                _ => panic!("Expected Text"),
            },
            _ => panic!("Expected Assistant"),
        }
    }
}

#[test]
fn test_stream_parser_iterator() {
    let lines = vec![
        r#"{"type":"system","session_id":"abc"}"#.to_string(),
        "".to_string(),
        r#"{"type":"result","total_cost_usd":0.01}"#.to_string(),
    ];

    let parser = StreamParser::new(lines.into_iter());
    let results: Vec<_> = parser.collect();

    assert_eq!(results.len(), 3);
    assert!(matches!(
        results[0],
        ParsedLine::Event(StreamEvent::System(_))
    ));
    assert!(matches!(results[1], ParsedLine::Empty));
    assert!(matches!(
        results[2],
        ParsedLine::Event(StreamEvent::Result(_))
    ));
}

#[test]
fn test_stream_parser_with_errors() {
    let lines = vec![
        r#"{"type":"system"}"#.to_string(),
        "invalid".to_string(),
        r#"{"type":"result"}"#.to_string(),
    ];

    let parser = StreamParser::new(lines.into_iter());
    let events: Vec<_> = parser
        .filter_map(|r| match r {
            ParsedLine::Event(e) => Some(e),
            _ => None,
        })
        .collect();

    assert_eq!(events.len(), 2);
}

#[test]
fn test_stream_parser_empty_iterator() {
    let lines: Vec<String> = vec![];
    let parser = StreamParser::new(lines.into_iter());
    let results: Vec<_> = parser.collect();
    assert!(results.is_empty());
}

#[test]
fn test_parse_stream_output_handles_incomplete_line_at_end() {
    // Simulating a stream that might be cut off - the incomplete line should error
    let output = r#"{"type":"system","session_id":"abc"}
{"type":"assistant","message":{"content":[{"type":"text","text":"Hello"#;

    let (events, errors) = parse_stream_output(output);
    assert_eq!(events.len(), 1);
    assert_eq!(errors.len(), 1);
}

#[test]
fn test_parse_stream_line_user_event() {
    let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_123","content":"file contents"}]}}"#;
    match parse_stream_line(line) {
        ParsedLine::Event(StreamEvent::User(usr)) => {
            assert_eq!(usr.message.content.len(), 1);
            assert_eq!(
                usr.message.content[0].tool_use_id,
                Some("toolu_123".to_string())
            );
        }
        other => panic!("Expected User event, got {:?}", other),
    }
}

#[test]
fn test_parse_real_claude_output_simulation() {
    // Simulate a realistic Claude stream-json output sequence
    let output = r#"{"type":"system","subtype":"init","session_id":"f5b6aaac-4316-454a","model":"claude-opus-4-5-20251101","tools":[{"name":"Read"},{"name":"Edit"}]}
{"type":"assistant","message":{"id":"msg_01ABC","content":[{"type":"text","text":"I'll help you implement this feature."}],"stop_reason":"end_turn"}}
{"type":"assistant","message":{"id":"msg_01DEF","content":[{"type":"text","text":"Let me read the file first."},{"type":"tool_use","id":"toolu_01XYZ","name":"Read","input":{"file_path":"/src/main.rs"}}],"stop_reason":"tool_use"}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_01XYZ","content":"fn main() { }"}]}}
{"type":"assistant","message":{"id":"msg_01GHI","content":[{"type":"text","text":"Done! The implementation is complete."}],"stop_reason":"end_turn"}}
{"type":"result","subtype":"success","total_cost_usd":0.226354,"duration_ms":40966,"num_turns":3,"usage":{"input_tokens":712,"output_tokens":2971}}"#;

    let (events, errors) = parse_stream_output(output);

    assert!(errors.is_empty(), "Parse errors: {:?}", errors);
    assert_eq!(events.len(), 6);

    // Verify the sequence
    match &events[0] {
        StreamEvent::System(sys) => {
            assert_eq!(sys.session_id, Some("f5b6aaac-4316-454a".to_string()));
            assert_eq!(sys.tools.len(), 2);
        }
        _ => panic!("Expected System"),
    }

    // Check result event
    match &events[5] {
        StreamEvent::Result(res) => {
            assert_eq!(res.total_cost_usd, Some(0.226354));
            assert_eq!(res.duration_ms, Some(40966));
            assert_eq!(res.num_turns, Some(3));
        }
        _ => panic!("Expected Result"),
    }
}
