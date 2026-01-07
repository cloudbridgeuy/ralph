//! Fence detection utilities for markdown code blocks.

/// Check if a line is an opening fence and extract the language hint.
///
/// Returns `Some(Some(lang))` if a language is specified, `Some(None)` if it's
/// a bare fence, or `None` if it's not a fence at all.
pub(crate) fn parse_fence_open(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim_start();

    // Check for triple backticks
    if let Some(after_fence) = trimmed.strip_prefix("```") {
        let after_fence = after_fence.trim();
        if after_fence.is_empty() {
            Some(None)
        } else {
            // Extract language (first word)
            let lang = after_fence.split_whitespace().next().unwrap_or("");
            Some(Some(lang.to_string()))
        }
    } else {
        None
    }
}

/// Check if a line is a closing fence.
pub(crate) fn is_fence_close(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "```"
}
