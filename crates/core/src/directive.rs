use std::sync::LazyLock;

use regex::Regex;

use crate::chunk::ParsedChunk;

/// Regex for matching directive opening tags: `<ralph-{verb} to="{target}">`.
/// Compiled once at first use since this is a compile-time constant pattern.
#[allow(clippy::expect_used)]
static OPEN_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<ralph-(\w+)\s+to="([^"]*)">"#)
        .expect("compile-time invariant: directive regex is valid")
});

/// The canonical directive verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveVerb {
    Ask,
    /// Non-blocking comment shown to the human.
    Comment,
}

/// Normalize a raw verb string to a canonical `DirectiveVerb`.
///
/// Synonym mapping:
/// - `ask`, `tell`, `say`, `respond`, `reply` -> `Ask`
///
/// Returns `None` for unknown verbs.
pub fn normalize_verb(raw: &str) -> Option<DirectiveVerb> {
    match raw {
        "ask" | "tell" | "say" | "respond" | "reply" => Some(DirectiveVerb::Ask),
        "comment" | "note" | "inform" => Some(DirectiveVerb::Comment),
        _ => None,
    }
}

/// Split text into alternating prose and directive chunks.
///
/// Scans left-to-right for directive tags, producing `ParsedChunk::prose()`
/// for text between directives and `ParsedChunk::directive()` for each
/// recognized directive. Directives with unknown verbs are left as prose.
///
/// If no directives are found, returns a single `ParsedChunk::prose(text)`.
pub fn split_text_around_directives(text: &str) -> Vec<ParsedChunk> {
    let mut chunks = Vec::new();
    let mut search_start = 0;

    while search_start < text.len() {
        let haystack = &text[search_start..];

        let cap = match OPEN_TAG_RE.captures(haystack) {
            Some(c) => c,
            None => break,
        };

        let full_match = match cap.get(0) {
            Some(m) => m,
            None => break,
        };
        let raw_verb = match cap.get(1) {
            Some(m) => m.as_str(),
            None => break,
        };
        let target = match cap.get(2) {
            Some(m) => m.as_str(),
            None => break,
        };

        let closing_tag = format!("</ralph-{raw_verb}>");
        let payload_start_in_haystack = full_match.end();
        let after_open = &haystack[payload_start_in_haystack..];

        match (after_open.find(&closing_tag), normalize_verb(raw_verb)) {
            (Some(end), Some(verb)) => {
                // Emit prose before this directive (if non-empty)
                let before = &haystack[..full_match.start()];
                if !before.is_empty() {
                    chunks.push(ParsedChunk::prose(before));
                }

                let payload = &after_open[..end];
                let canonical_verb = match verb {
                    DirectiveVerb::Ask => "ask",
                    DirectiveVerb::Comment => "comment",
                };
                chunks.push(ParsedChunk::directive(payload, canonical_verb, target));

                let close_end = payload_start_in_haystack + end + closing_tag.len();
                search_start += close_end;
            }
            (Some(end), None) => {
                // Unknown verb — skip past the entire tag pair, will be emitted as prose later
                let close_end = payload_start_in_haystack + end + closing_tag.len();
                search_start += close_end;
            }
            (None, Some(verb)) => {
                // Opening tag with no closing — consume all remaining text as directive
                let before = &haystack[..full_match.start()];
                if !before.is_empty() {
                    chunks.push(ParsedChunk::prose(before));
                }
                let payload = after_open;
                let canonical_verb = match verb {
                    DirectiveVerb::Ask => "ask",
                    DirectiveVerb::Comment => "comment",
                };
                chunks.push(ParsedChunk::directive(payload, canonical_verb, target));
                search_start = text.len(); // consumed everything
                break;
            }
            (None, None) => {
                // Unknown verb — skip past opening tag
                search_start += full_match.end();
            }
        }
    }

    // Emit any trailing text as prose
    if search_start < text.len() {
        chunks.push(ParsedChunk::prose(&text[search_start..]));
    }

    // If nothing was found, return a single prose chunk
    if chunks.is_empty() {
        chunks.push(ParsedChunk::prose(text));
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkType;

    // =========================================================================
    // normalize_verb tests
    // =========================================================================

    #[test]
    fn normalize_verb_canonical_ask() {
        assert_eq!(normalize_verb("ask"), Some(DirectiveVerb::Ask));
    }

    #[test]
    fn normalize_verb_ask_synonyms() {
        for synonym in &["tell", "say", "respond", "reply"] {
            assert_eq!(
                normalize_verb(synonym),
                Some(DirectiveVerb::Ask),
                "Expected Ask for synonym '{synonym}'"
            );
        }
    }

    #[test]
    fn normalize_verb_unknown_returns_none() {
        assert_eq!(normalize_verb("explode"), None);
        assert_eq!(normalize_verb(""), None);
        assert_eq!(normalize_verb("ASK"), None);
        assert_eq!(normalize_verb("Ask"), None);
    }

    // =========================================================================
    // split_text_around_directives tests
    // =========================================================================

    #[test]
    fn split_no_directives_returns_single_prose() {
        let chunks = split_text_around_directives("Just plain text.");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], ParsedChunk::prose("Just plain text."));
    }

    #[test]
    fn split_single_inline_directive() {
        let text = r#"Before <ralph-ask to="reviewer">Please review.</ralph-ask> After"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], ParsedChunk::prose("Before "));
        assert_eq!(
            chunks[1],
            ParsedChunk::directive("Please review.", "ask", "reviewer")
        );
        assert_eq!(chunks[2], ParsedChunk::prose(" After"));
    }

    #[test]
    fn split_multiple_directives() {
        let text =
            r#"A <ralph-ask to="a">Q1</ralph-ask> B <ralph-comment to="b">H1</ralph-comment> C"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunks[0], ParsedChunk::prose("A "));
        assert_eq!(chunks[1], ParsedChunk::directive("Q1", "ask", "a"));
        assert_eq!(chunks[2], ParsedChunk::prose(" B "));
        assert_eq!(chunks[3], ParsedChunk::directive("H1", "comment", "b"));
        assert_eq!(chunks[4], ParsedChunk::prose(" C"));
    }

    #[test]
    fn split_multiline_payload_preserved() {
        let text = "<ralph-ask to=\"r\">Line 1\nLine 2\nLine 3</ralph-ask>";
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            ParsedChunk::directive("Line 1\nLine 2\nLine 3", "ask", "r")
        );
    }

    #[test]
    fn split_unknown_verb_stays_as_prose() {
        let text = r#"<ralph-explode to="target">Boom</ralph-explode>"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 1);
        // The entire text is emitted as prose since the verb is unknown
        assert!(matches!(&chunks[0].chunk_type, ChunkType::Prose));
    }

    #[test]
    fn split_empty_payload_directive() {
        let text = r#"<ralph-ask to="target"></ralph-ask>"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], ParsedChunk::directive("", "ask", "target"));
    }

    #[test]
    fn split_directive_at_start_of_text() {
        let text = r#"<ralph-ask to="a">Q</ralph-ask> trailing"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], ParsedChunk::directive("Q", "ask", "a"));
        assert_eq!(chunks[1], ParsedChunk::prose(" trailing"));
    }

    #[test]
    fn split_directive_at_end_of_text() {
        let text = r#"leading <ralph-ask to="a">Q</ralph-ask>"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], ParsedChunk::prose("leading "));
        assert_eq!(chunks[1], ParsedChunk::directive("Q", "ask", "a"));
    }

    #[test]
    fn split_synonym_verbs_normalized() {
        let text = r#"<ralph-tell to="a">Hi</ralph-tell> <ralph-note to="b">Go</ralph-note>"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], ParsedChunk::directive("Hi", "ask", "a"));
        assert_eq!(chunks[1], ParsedChunk::prose(" "));
        assert_eq!(chunks[2], ParsedChunk::directive("Go", "comment", "b"));
    }

    // =========================================================================
    // split_text_around_directives — opening tag without closing tag
    // =========================================================================

    #[test]
    fn split_opening_without_closing_becomes_directive() {
        let text = r#"<ralph-ask to="reviewer">Please review this code."#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            chunks[0],
            ParsedChunk::directive("Please review this code.", "ask", "reviewer")
        );
    }

    #[test]
    fn split_prose_before_opening_without_closing() {
        let text = r#"Some preamble. <ralph-comment to="deployer">Deploy this now"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], ParsedChunk::prose("Some preamble. "));
        assert_eq!(
            chunks[1],
            ParsedChunk::directive("Deploy this now", "comment", "deployer")
        );
    }

    #[test]
    fn split_unknown_verb_without_closing_stays_prose() {
        let text = r#"Before <ralph-explode to="target">Boom and more text"#;
        let chunks = split_text_around_directives(text);
        assert_eq!(chunks.len(), 1);
        // Unknown verb is skipped; trailing text emitted as prose
        assert!(matches!(&chunks[0].chunk_type, ChunkType::Prose));
    }

    #[test]
    fn comment_verb_normalize_split() {
        // normalize_verb
        assert_eq!(normalize_verb("comment"), Some(DirectiveVerb::Comment));
        assert_eq!(normalize_verb("note"), Some(DirectiveVerb::Comment));
        assert_eq!(normalize_verb("inform"), Some(DirectiveVerb::Comment));

        // split_text_around_directives
        let c = split_text_around_directives(r#"A <ralph-comment to="h">B</ralph-comment> C"#);
        assert_eq!(c.len(), 3);
        assert_eq!(c[1], ParsedChunk::directive("B", "comment", "h"));
    }
}
