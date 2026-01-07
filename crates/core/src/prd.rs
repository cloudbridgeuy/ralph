//! PRD (Product Requirements Document) parsing and analysis.
//!
//! This module provides pure functions for parsing TOML PRD files and
//! counting pending stories. Following the Functional Core pattern,
//! all functions operate on data provided as arguments - no file I/O.

use serde::Deserialize;

/// Error type for PRD parsing operations.
#[derive(thiserror::Error, Debug)]
pub enum PrdError {
    /// TOML parsing failed
    #[error("Failed to parse PRD: {0}")]
    Parse(#[from] toml::de::Error),

    /// PRD contains no stories
    #[error("PRD contains no stories")]
    NoStories,
}

/// A single story from the PRD.
///
/// Only the `passes` field is extracted; all other fields are ignored
/// as specified in the architecture document.
#[derive(Debug, Deserialize)]
pub struct Story {
    /// Whether this story has been completed.
    /// Aliased to support both `passes` and `passed` field names.
    #[serde(alias = "passed", default)]
    pub passes: bool,
}

/// The parsed PRD document containing stories.
#[derive(Debug, Deserialize)]
pub struct Prd {
    /// The list of stories in the PRD.
    #[serde(default)]
    pub stories: Vec<Story>,
}

/// Result of parsing a PRD file.
#[derive(Debug)]
pub struct PrdAnalysis {
    /// Total number of stories in the PRD.
    pub total_stories: usize,
    /// Number of stories where `passes = false`.
    pub pending_count: usize,
    /// Number of stories where `passes = true`.
    pub completed_count: usize,
}

/// Parse PRD content and return analysis.
///
/// This is a pure function - it takes TOML content as a string and returns
/// the parsed analysis. File I/O is handled at the shell layer.
///
/// # Arguments
///
/// * `content` - The raw TOML content of the PRD file
///
/// # Returns
///
/// * `Ok(PrdAnalysis)` - Analysis of the PRD including pending story count
/// * `Err(PrdError::Parse)` - If TOML parsing fails
/// * `Err(PrdError::NoStories)` - If the PRD contains no `[[stories]]` entries
///
/// # Example
///
/// ```
/// use ralph_core::prd::parse_prd;
///
/// let content = r#"
/// [[stories]]
/// description = "First story"
/// passes = false
///
/// [[stories]]
/// description = "Second story"
/// passes = true
/// "#;
///
/// let analysis = parse_prd(content).unwrap();
/// assert_eq!(analysis.total_stories, 2);
/// assert_eq!(analysis.pending_count, 1);
/// assert_eq!(analysis.completed_count, 1);
/// ```
pub fn parse_prd(content: &str) -> Result<PrdAnalysis, PrdError> {
    let prd: Prd = toml::from_str(content)?;

    if prd.stories.is_empty() {
        return Err(PrdError::NoStories);
    }

    let total_stories = prd.stories.len();
    let completed_count = prd.stories.iter().filter(|s| s.passes).count();
    let pending_count = total_stories - completed_count;

    Ok(PrdAnalysis {
        total_stories,
        pending_count,
        completed_count,
    })
}

/// Count pending stories from PRD content.
///
/// Convenience function that returns just the pending count.
/// Useful when only the count is needed for iteration limits.
///
/// # Arguments
///
/// * `content` - The raw TOML content of the PRD file
///
/// # Returns
///
/// * `Ok(usize)` - Number of pending stories
/// * `Err(PrdError)` - If parsing fails or no stories exist
pub fn count_pending_stories(content: &str) -> Result<usize, PrdError> {
    parse_prd(content).map(|analysis| analysis.pending_count)
}

/// Detect if PRD content has changed.
///
/// This is a pure function that compares two PRD content strings byte-for-byte.
/// As specified in the architecture, the comparison is exact (not semantic) -
/// even whitespace changes count as changes.
///
/// # Arguments
///
/// * `before` - The PRD content before the iteration
/// * `after` - The PRD content after the iteration
///
/// # Returns
///
/// * `true` if the content has changed (differs byte-for-byte)
/// * `false` if the content is identical
///
/// # Example
///
/// ```
/// use ralph_core::prd::has_prd_changed;
///
/// let before = "[[stories]]\npasses = false\n";
/// let after = "[[stories]]\npasses = true\n";
/// assert!(has_prd_changed(before, after));
///
/// let same = "[[stories]]\npasses = false\n";
/// assert!(!has_prd_changed(before, same));
/// ```
pub fn has_prd_changed(before: &str, after: &str) -> bool {
    before != after
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prd_with_mixed_stories() {
        let content = r#"
[[stories]]
description = "First story"
passes = false

[[stories]]
description = "Second story"
passes = true

[[stories]]
description = "Third story"
passes = false
"#;

        let analysis = parse_prd(content).unwrap();
        assert_eq!(analysis.total_stories, 3);
        assert_eq!(analysis.pending_count, 2);
        assert_eq!(analysis.completed_count, 1);
    }

    #[test]
    fn test_parse_prd_all_pending() {
        let content = r#"
[[stories]]
passes = false

[[stories]]
passes = false
"#;

        let analysis = parse_prd(content).unwrap();
        assert_eq!(analysis.total_stories, 2);
        assert_eq!(analysis.pending_count, 2);
        assert_eq!(analysis.completed_count, 0);
    }

    #[test]
    fn test_parse_prd_all_completed() {
        let content = r#"
[[stories]]
passes = true

[[stories]]
passes = true
"#;

        let analysis = parse_prd(content).unwrap();
        assert_eq!(analysis.total_stories, 2);
        assert_eq!(analysis.pending_count, 0);
        assert_eq!(analysis.completed_count, 2);
    }

    #[test]
    fn test_parse_prd_no_stories_error() {
        let content = "# Empty PRD\n";
        let result = parse_prd(content);
        assert!(matches!(result, Err(PrdError::NoStories)));
    }

    #[test]
    fn test_parse_prd_malformed_toml() {
        let content = "this is not valid toml [[[";
        let result = parse_prd(content);
        assert!(matches!(result, Err(PrdError::Parse(_))));
    }

    #[test]
    fn test_parse_prd_ignores_extra_fields() {
        let content = r#"
[[stories]]
category = "functional"
description = "Some story"
steps = ["step 1", "step 2"]
passes = false
acceptance = ["criteria 1"]
extra_field = "ignored"
"#;

        let analysis = parse_prd(content).unwrap();
        assert_eq!(analysis.total_stories, 1);
        assert_eq!(analysis.pending_count, 1);
    }

    #[test]
    fn test_parse_prd_default_passes_to_false() {
        let content = r#"
[[stories]]
description = "Story without passes field"
"#;

        let analysis = parse_prd(content).unwrap();
        assert_eq!(analysis.pending_count, 1);
    }

    #[test]
    fn test_parse_prd_supports_passed_alias() {
        let content = r#"
[[stories]]
description = "Story with passed field"
passed = true
"#;

        let analysis = parse_prd(content).unwrap();
        assert_eq!(analysis.completed_count, 1);
    }

    #[test]
    fn test_count_pending_stories() {
        let content = r#"
[[stories]]
passes = false

[[stories]]
passes = true

[[stories]]
passes = false
"#;

        let count = count_pending_stories(content).unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_has_prd_changed_detects_content_change() {
        let before = r#"
[[stories]]
passes = false
"#;
        let after = r#"
[[stories]]
passes = true
"#;

        assert!(has_prd_changed(before, after));
    }

    #[test]
    fn test_has_prd_changed_identical_content() {
        let content = r#"
[[stories]]
passes = false

[[stories]]
passes = true
"#;

        assert!(!has_prd_changed(content, content));
    }

    #[test]
    fn test_has_prd_changed_whitespace_change() {
        let before = "[[stories]]\npasses = false\n";
        let after = "[[stories]]\npasses = false \n"; // Extra space at end

        // As per architecture spec: "Whitespace changes count as changes"
        assert!(has_prd_changed(before, after));
    }

    #[test]
    fn test_has_prd_changed_empty_strings() {
        assert!(!has_prd_changed("", ""));
    }

    #[test]
    fn test_has_prd_changed_different_story_count() {
        let before = r#"
[[stories]]
passes = false
"#;
        let after = r#"
[[stories]]
passes = false

[[stories]]
passes = true
"#;

        assert!(has_prd_changed(before, after));
    }

    #[test]
    fn test_has_prd_changed_comment_change() {
        let before = r#"
# Comment 1
[[stories]]
passes = false
"#;
        let after = r#"
# Comment 2
[[stories]]
passes = false
"#;

        // Comments are part of the content, so this counts as a change
        assert!(has_prd_changed(before, after));
    }

    #[test]
    fn test_has_prd_changed_newline_normalization() {
        let before = "[[stories]]\npasses = false\n";
        let after = "[[stories]]\r\npasses = false\r\n"; // Windows line endings

        // Different line endings = different content
        assert!(has_prd_changed(before, after));
    }
}
