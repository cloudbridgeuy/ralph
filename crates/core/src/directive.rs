//! Directive parsing and validation for multi-agent orchestration.
//!
//! Pure functions for extracting structured directives from persona output,
//! normalizing verb synonyms, and validating that directive sets are internally
//! consistent (all-asks or all-handovers, never mixed).

use std::sync::LazyLock;

use regex::Regex;

/// Regex for matching directive opening tags: `<ralph-{verb} to="{target}">`.
/// Compiled once at first use since this is a compile-time constant pattern.
#[allow(clippy::expect_used)]
static OPEN_TAG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<ralph-(\w+)\s+to="([^"]*)">"#)
        .expect("compile-time invariant: directive regex is valid")
});

/// Error type for directive validation operations.
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum DirectiveError {
    /// The directive set contains no directives.
    #[error("Directive set is empty")]
    Empty,

    /// The directive set mixes ask and handover verbs.
    #[error("Mixed directive verbs: set contains both ask and handover directives")]
    MixedVerbs,
}

/// The two canonical directive verbs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveVerb {
    Ask,
    Handover,
}

/// A parsed directive from persona output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub verb: DirectiveVerb,
    pub target: String,
    pub payload: String,
}

/// A validated set of directives — no mixed ask+handover.
/// Makes the impossible state (mixed verbs) unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidatedDirectiveSet {
    Asks(Vec<Directive>),
    Handovers(Vec<Directive>),
}

/// Normalize a raw verb string to a canonical `DirectiveVerb`.
///
/// Synonym mapping:
/// - `ask`, `tell`, `say`, `respond`, `reply` -> `Ask`
/// - `handover`, `delegate`, `pass`, `transfer` -> `Handover`
///
/// Returns `None` for unknown verbs.
pub fn normalize_verb(raw: &str) -> Option<DirectiveVerb> {
    match raw {
        "ask" | "tell" | "say" | "respond" | "reply" => Some(DirectiveVerb::Ask),
        "handover" | "delegate" | "pass" | "transfer" => Some(DirectiveVerb::Handover),
        _ => None,
    }
}

/// Parse directives from persona output text.
///
/// Scans left-to-right for top-level XML-style tags matching
/// `<ralph-{verb} to="{target}">{payload}</ralph-{verb}>`.
/// Payload content is treated as opaque plaintext — nested directive-like
/// tags inside a payload are NOT parsed as separate directives.
///
/// Each verb is normalized via [`normalize_verb`]; directives with unknown verbs are skipped.
pub fn parse_directives(text: &str) -> Vec<Directive> {
    let mut directives = Vec::new();
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
            Some(m) => m.as_str().to_string(),
            None => break,
        };

        let closing_tag = format!("</ralph-{raw_verb}>");
        let payload_start_in_haystack = full_match.end();
        let after_open = &haystack[payload_start_in_haystack..];

        match after_open.find(&closing_tag) {
            Some(end) => {
                let payload = after_open[..end].to_string();

                if let Some(verb) = normalize_verb(raw_verb) {
                    directives.push(Directive {
                        verb,
                        target,
                        payload,
                    });
                }

                // Resume scanning AFTER the closing tag
                let close_end_in_haystack = payload_start_in_haystack + end + closing_tag.len();
                search_start += close_end_in_haystack;
            }
            None => {
                // No closing tag — skip past this opening tag
                search_start += full_match.end();
            }
        }
    }

    directives
}

/// Validate that all directives in a set share the same verb family.
///
/// Returns `ValidatedDirectiveSet::Asks` if all are asks, or
/// `ValidatedDirectiveSet::Handovers` if all are handovers.
/// Errors on mixed verbs or an empty set.
pub fn validate_directive_set(
    directives: Vec<Directive>,
) -> Result<ValidatedDirectiveSet, DirectiveError> {
    if directives.is_empty() {
        return Err(DirectiveError::Empty);
    }

    let has_asks = directives.iter().any(|d| d.verb == DirectiveVerb::Ask);
    let has_handovers = directives.iter().any(|d| d.verb == DirectiveVerb::Handover);

    if has_asks && has_handovers {
        return Err(DirectiveError::MixedVerbs);
    }

    if has_asks {
        Ok(ValidatedDirectiveSet::Asks(directives))
    } else {
        Ok(ValidatedDirectiveSet::Handovers(directives))
    }
}

/// Format persona responses into a single aggregated prompt string.
///
/// Each entry in `responses` is a `(persona_name, response_text)` pair.
/// Responses are separated by `---` lines. No trailing separator is emitted.
pub fn aggregate_responses(responses: &[(&str, &str)]) -> String {
    responses
        .iter()
        .map(|(name, text)| format!("Response from {name}:\n\n{text}\n"))
        .collect::<Vec<_>>()
        .join("\n---\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // normalize_verb tests
    // =========================================================================

    #[test]
    fn normalize_verb_canonical_ask() {
        assert_eq!(normalize_verb("ask"), Some(DirectiveVerb::Ask));
    }

    #[test]
    fn normalize_verb_canonical_handover() {
        assert_eq!(normalize_verb("handover"), Some(DirectiveVerb::Handover));
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
    fn normalize_verb_handover_synonyms() {
        for synonym in &["delegate", "pass", "transfer"] {
            assert_eq!(
                normalize_verb(synonym),
                Some(DirectiveVerb::Handover),
                "Expected Handover for synonym '{synonym}'"
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
    // parse_directives tests
    // =========================================================================

    #[test]
    fn parse_single_ask() {
        let text = r#"<ralph-ask to="reviewer">Please review this code.</ralph-ask>"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].verb, DirectiveVerb::Ask);
        assert_eq!(directives[0].target, "reviewer");
        assert_eq!(directives[0].payload, "Please review this code.");
    }

    #[test]
    fn parse_single_handover() {
        let text = r#"<ralph-handover to="deployer">Deploy to production.</ralph-handover>"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].verb, DirectiveVerb::Handover);
        assert_eq!(directives[0].target, "deployer");
        assert_eq!(directives[0].payload, "Deploy to production.");
    }

    #[test]
    fn parse_multiple_asks() {
        let text = r#"
Some preamble text.
<ralph-ask to="alpha">First question?</ralph-ask>
Middle text.
<ralph-ask to="beta">Second question?</ralph-ask>
Trailing text.
"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].target, "alpha");
        assert_eq!(directives[0].payload, "First question?");
        assert_eq!(directives[1].target, "beta");
        assert_eq!(directives[1].payload, "Second question?");
    }

    #[test]
    fn parse_synonym_normalization() {
        let text = r#"
<ralph-tell to="a">Hello</ralph-tell>
<ralph-delegate to="b">Take over</ralph-delegate>
<ralph-say to="c">Greetings</ralph-say>
<ralph-pass to="d">Your turn</ralph-pass>
"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 4);
        assert_eq!(directives[0].verb, DirectiveVerb::Ask);
        assert_eq!(directives[0].target, "a");
        assert_eq!(directives[1].verb, DirectiveVerb::Handover);
        assert_eq!(directives[1].target, "b");
        assert_eq!(directives[2].verb, DirectiveVerb::Ask);
        assert_eq!(directives[2].target, "c");
        assert_eq!(directives[3].verb, DirectiveVerb::Handover);
        assert_eq!(directives[3].target, "d");
    }

    #[test]
    fn parse_no_directives_returns_empty() {
        let text = "Just some regular text with no directives.";
        let directives = parse_directives(text);
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_malformed_xml_skipped() {
        // Missing closing tag
        let text = r#"<ralph-ask to="target">No closing tag"#;
        let directives = parse_directives(text);
        assert!(directives.is_empty());

        // Mismatched tags
        let text = r#"<ralph-ask to="target">Mismatch</ralph-handover>"#;
        let directives = parse_directives(text);
        assert!(directives.is_empty());

        // Missing to attribute
        let text = r#"<ralph-ask>No target</ralph-ask>"#;
        let directives = parse_directives(text);
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_unknown_verb_skipped() {
        let text = r#"<ralph-explode to="target">Boom</ralph-explode>"#;
        let directives = parse_directives(text);
        assert!(directives.is_empty());
    }

    #[test]
    fn parse_nested_content_preserved() {
        let text = r#"<ralph-ask to="reviewer">Review this:
```rust
fn main() {
    println!("Hello");
}
```
Thanks!</ralph-ask>"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 1);
        assert!(directives[0].payload.contains("```rust"));
        assert!(directives[0].payload.contains("println!"));
        assert!(directives[0].payload.contains("Thanks!"));
    }

    #[test]
    fn parse_empty_payload() {
        let text = r#"<ralph-ask to="target"></ralph-ask>"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].payload, "");
    }

    #[test]
    fn parse_nested_directive_in_payload_treated_as_text() {
        // PM tells architect to use a directive — the inner tag is payload, not a real directive.
        let text = r#"<ralph-ask to="architect">Please ask the developer a question. Use a <ralph-ask to="developer"> directive to pose your question.</ralph-ask>"#;
        let directives = parse_directives(text);
        assert_eq!(
            directives.len(),
            1,
            "should find exactly one top-level directive"
        );
        assert_eq!(directives[0].target, "architect");
        assert!(
            directives[0]
                .payload
                .contains(r#"<ralph-ask to="developer">"#),
            "inner directive tag should be preserved as payload text"
        );
    }

    #[test]
    fn parse_nested_directive_with_own_closing_tag() {
        // Outer ask wraps an inner ask that has its own closing tag.
        let text = r#"<ralph-ask to="architect">Tell dev: <ralph-ask to="developer">What is X?</ralph-ask> and report back.</ralph-ask>"#;
        let directives = parse_directives(text);
        assert_eq!(
            directives.len(),
            1,
            "should find exactly one top-level directive"
        );
        assert_eq!(directives[0].target, "architect");
        // Payload runs from after outer open tag to the FIRST </ralph-ask> — which is the inner one.
        // This is acceptable: the outer directive's payload is truncated at the first matching close tag.
        // The key property is: we do NOT produce a second directive for "developer".
    }

    #[test]
    fn parse_sequential_directives_still_work() {
        // Two top-level directives side by side (not nested).
        let text = r#"<ralph-ask to="alpha">Question 1</ralph-ask> some text <ralph-ask to="beta">Question 2</ralph-ask>"#;
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].target, "alpha");
        assert_eq!(directives[0].payload, "Question 1");
        assert_eq!(directives[1].target, "beta");
        assert_eq!(directives[1].payload, "Question 2");
    }

    #[test]
    fn parse_directive_after_nested_is_found() {
        // An outer directive with a nested tag, followed by a separate top-level directive.
        let text = r#"<ralph-ask to="architect">Use <ralph-ask to="developer">Q</ralph-ask> please.</ralph-ask>
<ralph-handover to="deployer">Ship it.</ralph-handover>"#;
        // The outer ask's payload ends at the first </ralph-ask> (the inner close).
        // Then the parser resumes and finds the handover.
        let directives = parse_directives(text);
        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].verb, DirectiveVerb::Ask);
        assert_eq!(directives[0].target, "architect");
        assert_eq!(directives[1].verb, DirectiveVerb::Handover);
        assert_eq!(directives[1].target, "deployer");
    }

    // =========================================================================
    // validate_directive_set tests
    // =========================================================================

    #[test]
    fn validate_all_asks() {
        let directives = vec![
            Directive {
                verb: DirectiveVerb::Ask,
                target: "a".to_string(),
                payload: "q1".to_string(),
            },
            Directive {
                verb: DirectiveVerb::Ask,
                target: "b".to_string(),
                payload: "q2".to_string(),
            },
        ];
        let result = validate_directive_set(directives);
        assert!(matches!(result, Ok(ValidatedDirectiveSet::Asks(_))));
        if let Ok(ValidatedDirectiveSet::Asks(asks)) = result {
            assert_eq!(asks.len(), 2);
        }
    }

    #[test]
    fn validate_all_handovers() {
        let directives = vec![
            Directive {
                verb: DirectiveVerb::Handover,
                target: "a".to_string(),
                payload: "task1".to_string(),
            },
            Directive {
                verb: DirectiveVerb::Handover,
                target: "b".to_string(),
                payload: "task2".to_string(),
            },
        ];
        let result = validate_directive_set(directives);
        assert!(matches!(result, Ok(ValidatedDirectiveSet::Handovers(_))));
        if let Ok(ValidatedDirectiveSet::Handovers(handovers)) = result {
            assert_eq!(handovers.len(), 2);
        }
    }

    #[test]
    fn validate_mixed_verbs_error() {
        let directives = vec![
            Directive {
                verb: DirectiveVerb::Ask,
                target: "a".to_string(),
                payload: "question".to_string(),
            },
            Directive {
                verb: DirectiveVerb::Handover,
                target: "b".to_string(),
                payload: "task".to_string(),
            },
        ];
        let result = validate_directive_set(directives);
        assert_eq!(result, Err(DirectiveError::MixedVerbs));
    }

    #[test]
    fn validate_empty_set_error() {
        let result = validate_directive_set(vec![]);
        assert_eq!(result, Err(DirectiveError::Empty));
    }

    // =========================================================================
    // aggregate_responses tests
    // =========================================================================

    #[test]
    fn aggregate_single_response() {
        let responses = vec![("reviewer", "Looks good to me.")];
        let result = aggregate_responses(&responses);
        let expected = "Response from reviewer:\n\nLooks good to me.\n";
        assert_eq!(result, expected);
    }

    #[test]
    fn aggregate_multiple_responses() {
        let responses = vec![("alpha", "First response."), ("beta", "Second response.")];
        let result = aggregate_responses(&responses);
        let expected = "\
Response from alpha:

First response.

---

Response from beta:

Second response.
";
        assert_eq!(result, expected);
    }

    #[test]
    fn aggregate_empty_response_text() {
        let responses = vec![("silent", "")];
        let result = aggregate_responses(&responses);
        let expected = "Response from silent:\n\n\n";
        assert_eq!(result, expected);
    }

    #[test]
    fn aggregate_empty_responses_vec() {
        let responses: Vec<(&str, &str)> = vec![];
        let result = aggregate_responses(&responses);
        assert_eq!(result, "");
    }

    #[test]
    fn aggregate_three_responses_no_trailing_separator() {
        let responses = vec![("a", "one"), ("b", "two"), ("c", "three")];
        let result = aggregate_responses(&responses);
        // Verify no trailing --- separator
        assert!(!result.ends_with("---\n"));
        assert!(!result.ends_with("---"));
        // Verify structure
        assert!(result.contains("Response from a:"));
        assert!(result.contains("Response from b:"));
        assert!(result.contains("Response from c:"));
        // Should have exactly 2 separators for 3 responses
        assert_eq!(result.matches("---").count(), 2);
    }
}
