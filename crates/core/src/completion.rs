//! Completion detection for iteration loops
//!
//! This module provides pure functions for detecting when an iteration loop
//! should complete based on pending story count or completion markers in output.

/// Reasons why an iteration loop should complete
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionReason {
    /// All stories in the PRD have been completed (zero pending stories)
    AllStoriesComplete,
    /// The completion marker was found in the LLM output
    MarkerFound,
}

/// Checks if the iteration loop should complete based on pending story count
/// and the presence of a completion marker in the output.
///
/// # Arguments
///
/// * `pending_count` - Number of stories in the PRD with `passed = false`
/// * `output` - The LLM output text to scan for the completion marker
/// * `marker` - The completion marker string to search for (e.g., "<promise>COMPLETE</promise>")
///
/// # Returns
///
/// * `Some(CompletionReason)` if completion should occur
/// * `None` if the loop should continue
///
/// # Examples
///
/// ```
/// use ralph_core::completion::{check_completion, CompletionReason};
///
/// // Loop should complete when no stories remain
/// assert_eq!(
///     check_completion(0, "Some output", "<promise>COMPLETE</promise>"),
///     Some(CompletionReason::AllStoriesComplete)
/// );
///
/// // Loop should complete when marker is found
/// assert_eq!(
///     check_completion(5, "Work done <promise>COMPLETE</promise>", "<promise>COMPLETE</promise>"),
///     Some(CompletionReason::MarkerFound)
/// );
///
/// // Loop should continue when stories remain and no marker
/// assert_eq!(
///     check_completion(3, "Still working...", "<promise>COMPLETE</promise>"),
///     None
/// );
/// ```
pub fn check_completion(
    pending_count: usize,
    output: &str,
    marker: &str,
) -> Option<CompletionReason> {
    // Check for zero pending stories first (higher priority)
    if pending_count == 0 {
        return Some(CompletionReason::AllStoriesComplete);
    }

    // Check for completion marker in output
    if output.contains(marker) {
        return Some(CompletionReason::MarkerFound);
    }

    // Loop should continue
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_when_no_pending_stories() {
        let result = check_completion(0, "Some output", "<promise>COMPLETE</promise>");
        assert_eq!(result, Some(CompletionReason::AllStoriesComplete));
    }

    #[test]
    fn test_completion_when_marker_found() {
        let output = "I've completed all the work. <promise>COMPLETE</promise>";
        let result = check_completion(5, output, "<promise>COMPLETE</promise>");
        assert_eq!(result, Some(CompletionReason::MarkerFound));
    }

    #[test]
    fn test_completion_marker_in_middle_of_text() {
        let output = "Starting work\n<promise>COMPLETE</promise>\nSome trailing text";
        let result = check_completion(3, output, "<promise>COMPLETE</promise>");
        assert_eq!(result, Some(CompletionReason::MarkerFound));
    }

    #[test]
    fn test_no_completion_when_stories_remain_and_no_marker() {
        let result = check_completion(3, "Still working on features", "<promise>COMPLETE</promise>");
        assert_eq!(result, None);
    }

    #[test]
    fn test_no_completion_with_empty_output() {
        let result = check_completion(5, "", "<promise>COMPLETE</promise>");
        assert_eq!(result, None);
    }

    #[test]
    fn test_custom_marker() {
        let output = "Work finished [DONE]";
        let result = check_completion(2, output, "[DONE]");
        assert_eq!(result, Some(CompletionReason::MarkerFound));
    }

    #[test]
    fn test_marker_case_sensitive() {
        let output = "<PROMISE>COMPLETE</PROMISE>";
        let result = check_completion(1, output, "<promise>COMPLETE</promise>");
        assert_eq!(result, None);
    }

    #[test]
    fn test_zero_pending_takes_priority_over_marker() {
        // When both conditions are true, zero pending stories should be returned
        let output = "All done <promise>COMPLETE</promise>";
        let result = check_completion(0, output, "<promise>COMPLETE</promise>");
        assert_eq!(result, Some(CompletionReason::AllStoriesComplete));
    }

    #[test]
    fn test_partial_marker_not_matched() {
        let output = "In progress <promise>COMPLETE";
        let result = check_completion(2, output, "<promise>COMPLETE</promise>");
        assert_eq!(result, None);
    }

    #[test]
    fn test_marker_with_surrounding_whitespace() {
        let output = "Done\n  <promise>COMPLETE</promise>  \n";
        let result = check_completion(1, output, "<promise>COMPLETE</promise>");
        assert_eq!(result, Some(CompletionReason::MarkerFound));
    }
}
