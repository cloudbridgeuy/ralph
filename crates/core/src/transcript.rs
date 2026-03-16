//! Transcript types and formatting for conversation-loop strategies.
//!
//! Pure functions for building editor content, parsing human responses,
//! and formatting persona prompts. Following the Functional Core pattern,
//! all functions operate on data provided as arguments — no I/O.

/// Who spoke this entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Speaker {
    /// A named persona (agent).
    Persona(String),
    /// The human operator.
    Human,
}

/// A single conversation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub content: String,
}

/// What the human wrote (or didn't) in $EDITOR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HumanResponse {
    /// The human provided content.
    Content(String),
    /// The human aborted (empty or missing response).
    Abort,
}

/// Should the outer loop continue?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAction {
    Continue,
    Exit,
}

/// What happened at a comment soft-block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentResponse {
    /// Human pressed Enter without typing anything.
    Continue,
    /// Human typed a response.
    Reply(String),
}

/// Separator line shown in the editor between transcript and input area.
pub const EDITOR_SEPARATOR: &str = "--- Write your response below this line ---";

/// Format transcript entries + separator into editor file content.
///
/// Renders each transcript entry with a speaker label, followed by the
/// separator line and an empty area for the human to write in.
pub fn build_editor_content(transcript: &[TranscriptEntry]) -> String {
    let mut content = String::new();

    for entry in transcript {
        let label = match &entry.speaker {
            Speaker::Persona(name) => format!("[{name}]"),
            Speaker::Human => "[You]".to_string(),
        };
        content.push_str(&label);
        content.push('\n');
        content.push_str(&entry.content);
        content.push_str("\n\n");
    }

    content.push_str(EDITOR_SEPARATOR);
    content.push('\n');

    content
}

/// Extract human response text below the separator, or Abort if empty.
///
/// Looks for the separator line in the file content. If found, takes
/// everything after it. If the text below the separator is empty or
/// whitespace-only, returns `Abort`. If no separator is found, treats
/// the entire content as the response.
pub fn parse_editor_response(file_content: &str) -> HumanResponse {
    let below = match file_content.split_once(EDITOR_SEPARATOR) {
        Some((_, after)) => after,
        None => file_content,
    };

    let trimmed = below.trim();
    if trimmed.is_empty() {
        HumanResponse::Abort
    } else {
        HumanResponse::Content(trimmed.to_string())
    }
}

/// Determine loop action from human response.
pub fn check_exit(response: &HumanResponse) -> LoopAction {
    match response {
        HumanResponse::Content(_) => LoopAction::Continue,
        HumanResponse::Abort => LoopAction::Exit,
    }
}

/// Build the prompt string for persona invocation from transcript + new human input.
///
/// Formats the conversation history into a structured prompt that gives
/// the persona full context of the conversation so far.
pub fn build_persona_prompt(transcript: &[TranscriptEntry], human_input: &str) -> String {
    if transcript.is_empty() {
        return human_input.to_string();
    }

    let mut parts = vec!["<conversation_history>".to_string()];

    for entry in transcript {
        let label = match &entry.speaker {
            Speaker::Persona(name) => format!("[{name}]"),
            Speaker::Human => "[Human]".to_string(),
        };
        parts.push(format!("{label}: {}", entry.content));
        parts.push(String::new());
    }

    parts.push("</conversation_history>".to_string());
    parts.push(String::new());
    parts.push(format!("[Human]: {human_input}"));

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // build_editor_content tests
    // =========================================================================

    #[test]
    fn build_editor_content_empty_transcript() {
        let content = build_editor_content(&[]);
        assert_eq!(content, format!("{EDITOR_SEPARATOR}\n"));
    }

    #[test]
    fn build_editor_content_single_human_entry() {
        let transcript = vec![TranscriptEntry {
            speaker: Speaker::Human,
            content: "Hello world".to_string(),
        }];
        let content = build_editor_content(&transcript);
        assert!(content.contains("[You]"));
        assert!(content.contains("Hello world"));
        assert!(content.ends_with(&format!("{EDITOR_SEPARATOR}\n")));
    }

    #[test]
    fn build_editor_content_single_persona_entry() {
        let transcript = vec![TranscriptEntry {
            speaker: Speaker::Persona("storyteller".to_string()),
            content: "Once upon a time...".to_string(),
        }];
        let content = build_editor_content(&transcript);
        assert!(content.contains("[storyteller]"));
        assert!(content.contains("Once upon a time..."));
    }

    #[test]
    fn build_editor_content_multiple_entries() {
        let transcript = vec![
            TranscriptEntry {
                speaker: Speaker::Human,
                content: "Write a story".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Persona("storyteller".to_string()),
                content: "It was a dark night.".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Human,
                content: "Continue".to_string(),
            },
        ];
        let content = build_editor_content(&transcript);
        assert!(content.contains("[You]\nWrite a story"));
        assert!(content.contains("[storyteller]\nIt was a dark night."));
        assert!(content.contains("[You]\nContinue"));
        assert!(content.ends_with(&format!("{EDITOR_SEPARATOR}\n")));
    }

    #[test]
    fn build_editor_content_mixed_speakers() {
        let transcript = vec![
            TranscriptEntry {
                speaker: Speaker::Human,
                content: "Question".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Persona("alpha".to_string()),
                content: "Answer from alpha".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Persona("beta".to_string()),
                content: "Answer from beta".to_string(),
            },
        ];
        let content = build_editor_content(&transcript);
        assert!(content.contains("[You]"));
        assert!(content.contains("[alpha]"));
        assert!(content.contains("[beta]"));
    }

    // =========================================================================
    // parse_editor_response tests
    // =========================================================================

    #[test]
    fn parse_editor_response_content_below_separator() {
        let content = format!("Previous text\n{EDITOR_SEPARATOR}\nMy response here");
        assert_eq!(
            parse_editor_response(&content),
            HumanResponse::Content("My response here".to_string())
        );
    }

    #[test]
    fn parse_editor_response_empty_below_separator() {
        let content = format!("Previous text\n{EDITOR_SEPARATOR}\n");
        assert_eq!(parse_editor_response(&content), HumanResponse::Abort);
    }

    #[test]
    fn parse_editor_response_whitespace_only_below_separator() {
        let content = format!("Previous text\n{EDITOR_SEPARATOR}\n   \n  \n");
        assert_eq!(parse_editor_response(&content), HumanResponse::Abort);
    }

    #[test]
    fn parse_editor_response_no_separator() {
        let content = "Some content without separator";
        assert_eq!(
            parse_editor_response(content),
            HumanResponse::Content("Some content without separator".to_string())
        );
    }

    #[test]
    fn parse_editor_response_no_separator_empty() {
        assert_eq!(parse_editor_response(""), HumanResponse::Abort);
    }

    #[test]
    fn parse_editor_response_multiline_below_separator() {
        let content = format!("{EDITOR_SEPARATOR}\nLine 1\nLine 2\nLine 3");
        assert_eq!(
            parse_editor_response(&content),
            HumanResponse::Content("Line 1\nLine 2\nLine 3".to_string())
        );
    }

    // =========================================================================
    // check_exit tests
    // =========================================================================

    #[test]
    fn check_exit_content_continues() {
        assert_eq!(
            check_exit(&HumanResponse::Content("text".to_string())),
            LoopAction::Continue
        );
    }

    #[test]
    fn check_exit_abort_exits() {
        assert_eq!(check_exit(&HumanResponse::Abort), LoopAction::Exit);
    }

    // =========================================================================
    // build_persona_prompt tests
    // =========================================================================

    #[test]
    fn build_persona_prompt_empty_transcript() {
        let result = build_persona_prompt(&[], "Hello agent");
        assert_eq!(result, "Hello agent");
    }

    #[test]
    fn build_persona_prompt_with_history() {
        let transcript = vec![
            TranscriptEntry {
                speaker: Speaker::Human,
                content: "First message".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Persona("storyteller".to_string()),
                content: "First response".to_string(),
            },
        ];
        let result = build_persona_prompt(&transcript, "Second message");
        assert!(result.contains("<conversation_history>"));
        assert!(result.contains("[Human]: First message"));
        assert!(result.contains("[storyteller]: First response"));
        assert!(result.contains("</conversation_history>"));
        assert!(result.contains("[Human]: Second message"));
    }

    #[test]
    fn build_persona_prompt_multiple_turns() {
        let transcript = vec![
            TranscriptEntry {
                speaker: Speaker::Human,
                content: "Turn 1".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Persona("agent".to_string()),
                content: "Response 1".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Human,
                content: "Turn 2".to_string(),
            },
            TranscriptEntry {
                speaker: Speaker::Persona("agent".to_string()),
                content: "Response 2".to_string(),
            },
        ];
        let result = build_persona_prompt(&transcript, "Turn 3");
        assert!(result.contains("[Human]: Turn 1"));
        assert!(result.contains("[agent]: Response 1"));
        assert!(result.contains("[Human]: Turn 2"));
        assert!(result.contains("[agent]: Response 2"));
        assert!(result.contains("[Human]: Turn 3"));
        // History tags should wrap the transcript
        assert!(result.starts_with("<conversation_history>"));
    }
}
