use crate::stream::*;

#[test]
fn test_parse_chunks_from_events_prose_only() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Hello, world!\nThis is prose.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks = parse_chunks_from_events(&events);
    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert_eq!(chunks[0].content, "Hello, world!\nThis is prose.");
}

#[test]
fn test_parse_chunks_from_events_with_code_block() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Here's the code:\n\n```rust\nfn main() {}\n```\n\nDone!".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks = parse_chunks_from_events(&events);
    assert_eq!(chunks.len(), 3);

    // First chunk: prose
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert!(chunks[0].content.contains("Here's the code:"));

    // Second chunk: code
    match &chunks[1].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert_eq!(chunks[1].content, "fn main() {}");

    // Third chunk: prose
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
    assert!(chunks[2].content.contains("Done!"));
}

#[test]
fn test_parse_chunks_from_events_with_diff_block() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "Changes:\n\n```diff\n-old\n+new\n```\n\nApplied.".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks = parse_chunks_from_events(&events);
    assert_eq!(chunks.len(), 3);

    // First chunk: prose
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));

    // Second chunk: diff
    assert!(matches!(chunks[1].chunk_type, ChunkType::Diff));
    assert_eq!(chunks[1].content, "-old\n+new");

    // Third chunk: prose
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
}

#[test]
fn test_parse_chunks_from_events_multiple_code_blocks() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text:
                    "```python\nprint('hello')\n```\n\nand\n\n```javascript\nconsole.log('hi')\n```"
                        .to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks = parse_chunks_from_events(&events);
    assert_eq!(chunks.len(), 3);

    // First code block
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("python"));
        }
        _ => panic!("Expected code chunk"),
    }

    // Prose between
    assert!(matches!(chunks[1].chunk_type, ChunkType::Prose));

    // Second code block
    match &chunks[2].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("javascript"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_parse_chunks_from_events_empty() {
    let events: Vec<StreamEvent> = vec![];
    let chunks = parse_chunks_from_events(&events);
    assert!(chunks.is_empty());
}

#[test]
fn test_parse_chunks_from_events_non_assistant_events() {
    let events = vec![
        StreamEvent::System(SystemEvent {
            subtype: Some("init".to_string()),
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

    let chunks = parse_chunks_from_events(&events);
    assert!(chunks.is_empty());
}

#[test]
fn test_parse_chunks_from_events_multiple_assistant_events() {
    let events = vec![
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First part\n\n```rust\nfn a() {}\n```".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
        StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "\n\nSecond part\n\n```python\ndef b():\n    pass\n```".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        }),
    ];

    let chunks = parse_chunks_from_events(&events);
    // Should have: prose, rust code, prose, python code
    assert_eq!(chunks.len(), 4);

    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    match &chunks[1].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected rust code chunk"),
    }
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
    match &chunks[3].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("python"));
        }
        _ => panic!("Expected python code chunk"),
    }
}

#[test]
fn test_parse_chunks_from_events_with_heuristics_unfenced_diff() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

    let chunks = parse_chunks_from_events_with_heuristics(&events);
    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
}

#[test]
fn test_parse_chunks_from_events_with_heuristics_fenced_code_preserved() {
    // Fenced code blocks should still be detected correctly
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "```rust\nfn main() {}\n```".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks = parse_chunks_from_events_with_heuristics(&events);
    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
}

#[test]
fn test_parse_text_into_chunks_code_block() {
    let text = "Here's code:\n\n```rust\nfn main() {}\n```\n\nDone!";
    let chunks = parse_text_into_chunks(text);

    assert_eq!(chunks.len(), 3);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    match &chunks[1].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("rust"));
        }
        _ => panic!("Expected code chunk"),
    }
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
}

#[test]
fn test_parse_text_into_chunks_with_heuristics() {
    let text = "diff --git a/f.rs b/f.rs\n-old\n+new";
    let chunks = parse_text_into_chunks_with_heuristics(text);

    assert_eq!(chunks.len(), 1);
    assert!(matches!(chunks[0].chunk_type, ChunkType::Diff));
}

#[test]
fn test_parse_chunks_preserves_language_metadata() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: "```typescript\nconst x: number = 1;\n```".to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks = parse_chunks_from_events(&events);
    assert_eq!(chunks.len(), 1);
    match &chunks[0].chunk_type {
        ChunkType::Code { language } => {
            assert_eq!(language.as_deref(), Some("typescript"));
        }
        _ => panic!("Expected code chunk with language"),
    }
}

#[test]
fn test_parse_chunks_prose_between_code_blocks_preserved() {
    let events = vec![StreamEvent::Assistant(AssistantEvent {
            message: AssistantMessage {
                id: Some("msg_01".to_string()),
                content: vec![ContentBlock::Text {
                    text: "First\n\n```rust\nfn a() {}\n```\n\nMiddle text here\n\n```python\ndef b(): pass\n```\n\nLast".to_string(),
                }],
                model: None,
                stop_reason: None,
            },
        })];

    let chunks = parse_chunks_from_events(&events);
    assert_eq!(chunks.len(), 5);

    // Prose - code - prose - code - prose
    assert!(matches!(chunks[0].chunk_type, ChunkType::Prose));
    assert!(matches!(chunks[1].chunk_type, ChunkType::Code { .. }));
    assert!(matches!(chunks[2].chunk_type, ChunkType::Prose));
    assert!(chunks[2].content.contains("Middle text here"));
    assert!(matches!(chunks[3].chunk_type, ChunkType::Code { .. }));
    assert!(matches!(chunks[4].chunk_type, ChunkType::Prose));
}

#[test]
fn test_parse_chunks_matches_plain_text_parsing() {
    // Verify that parsing from events produces the same result as parsing plain text directly
    let text = "Intro\n\n```rust\nfn main() {\n    println!(\"Hello\");\n}\n```\n\nOutro";

    let events = vec![StreamEvent::Assistant(AssistantEvent {
        message: AssistantMessage {
            id: Some("msg_01".to_string()),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            model: None,
            stop_reason: None,
        },
    })];

    let chunks_from_events = parse_chunks_from_events(&events);
    let chunks_from_text = parse_text_into_chunks(text);

    assert_eq!(chunks_from_events.len(), chunks_from_text.len());
    for (a, b) in chunks_from_events.iter().zip(chunks_from_text.iter()) {
        assert_eq!(a.chunk_type, b.chunk_type);
        assert_eq!(a.content, b.content);
    }
}
