//! Diff highlighting with fallback chain (Imperative Shell).
//!
//! This module provides diff highlighting functionality using a fallback chain:
//! 1. Try `delta --paging=never` (best quality, feature-rich)
//! 2. Fall back to basic inline +/- line coloring (works without external tools)
//!
//! # Features
//!
//! - Automatic detection of available tools
//! - Graceful degradation when tools are unavailable
//! - Terminal detection for automatic color support
//! - ANSI color output for terminal display
//!
//! # Example
//!
//! ```no_run
//! use ralph::diff_highlight::highlight_diff;
//!
//! let diff = r#"
//! --- a/file.rs
//! +++ b/file.rs
//! @@ -1,3 +1,4 @@
//!  fn main() {
//! +    println!("Hello");
//!  }
//! "#;
//!
//! let highlighted = highlight_diff(diff);
//! print!("{}", highlighted);
//! ```

use crate::diff_theme::DiffColors;
use std::io::IsTerminal;
use std::process::{Command, Stdio};

/// Check if delta is available on the system.
///
/// Uses `which delta` to determine if delta is installed and accessible.
///
/// # Returns
///
/// `true` if delta is available, `false` otherwise.
///
/// # Example
///
/// ```no_run
/// use ralph::diff_highlight::is_delta_available;
///
/// if is_delta_available() {
///     println!("Delta is installed");
/// }
/// ```
pub fn is_delta_available() -> bool {
    Command::new("which")
        .arg("delta")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Highlight a diff using delta.
///
/// Pipes the diff through `delta --paging=never` to get colorized output.
/// Returns `None` if delta is not available or fails.
///
/// # Arguments
///
/// * `diff` - The diff content to highlight
///
/// # Returns
///
/// `Some(highlighted)` on success, `None` if delta failed or is unavailable.
///
/// # Example
///
/// ```no_run
/// use ralph::diff_highlight::highlight_with_delta;
///
/// let diff = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new";
/// if let Some(highlighted) = highlight_with_delta(diff) {
///     print!("{}", highlighted);
/// }
/// ```
pub fn highlight_with_delta(diff: &str) -> Option<String> {
    let mut child = Command::new("delta")
        .arg("--paging=never")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // Write diff to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(diff.as_bytes()).ok()?;
        // stdin is dropped here, closing the pipe
    }

    let output = child.wait_with_output().ok()?;

    if output.status.success() {
        String::from_utf8(output.stdout).ok()
    } else {
        None
    }
}

/// Apply basic inline coloring to a diff.
///
/// Colors lines based on their prefix:
/// - Lines starting with `+` (but not `+++`) are green (additions)
/// - Lines starting with `-` (but not `---`) are red (deletions)
/// - Lines starting with `@@` are cyan (hunk headers)
/// - Lines starting with `diff`, `index`, `---`, `+++` are bold (file headers)
/// - Other lines are unchanged
///
/// # Arguments
///
/// * `diff` - The diff content to highlight
///
/// # Returns
///
/// A string with ANSI color codes for terminal display.
///
/// # Example
///
/// ```
/// use ralph::diff_highlight::highlight_with_basic_colors;
/// use ralph::diff_theme::DiffColors;
///
/// let diff = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new";
/// let highlighted = highlight_with_basic_colors(diff, &DiffColors::default());
/// assert!(highlighted.contains("\x1b[")); // Contains ANSI codes
/// ```
pub fn highlight_with_basic_colors(diff: &str, colors: &DiffColors) -> String {
    use crate::ansi::{fg, BOLD, RESET};

    let mut output = String::with_capacity(diff.len() * 2);

    for line in diff.lines() {
        let colored_line = if line.starts_with("+++") || line.starts_with("---") {
            format!("{}{}{}", BOLD, line, RESET)
        } else if line.starts_with('+') {
            format!("{}{}{}", fg(Some(colors.add_fg)), line, RESET)
        } else if line.starts_with('-') {
            format!("{}{}{}", fg(Some(colors.del_fg)), line, RESET)
        } else if line.starts_with("@@") {
            format!("{}{}{}", fg(Some(colors.hunk_fg)), line, RESET)
        } else if line.starts_with("diff ") || line.starts_with("index ") {
            format!("{}{}{}", BOLD, line, RESET)
        } else {
            line.to_string()
        };

        output.push_str(&colored_line);
        output.push('\n');
    }

    output
}

/// Check if diff highlighting is supported in the current environment.
///
/// Returns `true` if stdout is connected to a terminal that supports
/// ANSI escape codes. Returns `false` if output is piped or redirected.
///
/// # Example
///
/// ```no_run
/// use ralph::diff_highlight::is_highlighting_supported;
///
/// if is_highlighting_supported() {
///     // Use colored output
/// } else {
///     // Plain text output
/// }
/// ```
pub fn is_highlighting_supported() -> bool {
    std::io::stdout().is_terminal()
}

/// Highlight a diff using the best available method.
///
/// Uses a fallback chain:
/// 1. Try delta (if available)
/// 2. Fall back to basic inline coloring
///
/// If terminal highlighting is not supported (piped output),
/// returns the diff unchanged.
///
/// # Arguments
///
/// * `diff` - The diff content to highlight
///
/// # Returns
///
/// A string with appropriate highlighting for the terminal.
///
/// # Example
///
/// ```no_run
/// use ralph::diff_highlight::highlight_diff;
///
/// let diff = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new";
/// let highlighted = highlight_diff(diff);
/// print!("{}", highlighted);
/// ```
pub fn highlight_diff(diff: &str) -> String {
    // If not a terminal, return unchanged
    if !is_highlighting_supported() {
        return diff.to_string();
    }

    // Try delta first (best quality)
    if is_delta_available() {
        if let Some(highlighted) = highlight_with_delta(diff) {
            return highlighted;
        }
    }

    // Fall back to basic coloring (always available)
    highlight_with_basic_colors(diff, &DiffColors::default())
}

/// A diff highlighter that caches tool availability checks.
///
/// Use this for multiple highlighting operations to avoid
/// repeated availability checks.
pub struct DiffHighlighter {
    delta_available: bool,
}

impl Default for DiffHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffHighlighter {
    /// Create a new diff highlighter, checking tool availability.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ralph::diff_highlight::DiffHighlighter;
    ///
    /// let highlighter = DiffHighlighter::new();
    /// ```
    pub fn new() -> Self {
        Self {
            delta_available: is_delta_available(),
        }
    }

    /// Check if delta is available.
    ///
    /// Returns the cached availability status.
    pub fn is_delta_available(&self) -> bool {
        self.delta_available
    }

    /// Highlight a diff using the best available method.
    ///
    /// # Arguments
    ///
    /// * `diff` - The diff content to highlight
    ///
    /// # Returns
    ///
    /// A string with appropriate highlighting.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ralph::diff_highlight::DiffHighlighter;
    ///
    /// let highlighter = DiffHighlighter::new();
    /// let diff = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new";
    /// let highlighted = highlighter.highlight(diff);
    /// print!("{}", highlighted);
    /// ```
    pub fn highlight(&self, diff: &str) -> String {
        // If not a terminal, return unchanged
        if !is_highlighting_supported() {
            return diff.to_string();
        }

        // Try delta first (best quality)
        if self.delta_available {
            if let Some(highlighted) = highlight_with_delta(diff) {
                return highlighted;
            }
        }

        // Fall back to basic coloring (always available)
        highlight_with_basic_colors(diff, &DiffColors::default())
    }

    /// Highlight a diff, bypassing terminal detection.
    ///
    /// Always applies highlighting, even if stdout is not a terminal.
    /// Useful for testing or when output will be displayed later.
    ///
    /// # Arguments
    ///
    /// * `diff` - The diff content to highlight
    ///
    /// # Returns
    ///
    /// A string with highlighting applied.
    pub fn highlight_always(&self, diff: &str) -> String {
        // Try delta first (best quality)
        if self.delta_available {
            if let Some(highlighted) = highlight_with_delta(diff) {
                return highlighted;
            }
        }

        // Fall back to basic coloring (always available)
        highlight_with_basic_colors(diff, &DiffColors::default())
    }
}

/// Format a diff for terminal output.
///
/// Convenience function that combines terminal detection with highlighting.
/// If terminal highlighting is not supported, returns the diff unchanged.
///
/// # Arguments
///
/// * `diff` - The diff content to format
///
/// # Returns
///
/// A string formatted for terminal display.
///
/// # Example
///
/// ```no_run
/// use ralph::diff_highlight::format_diff_for_terminal;
///
/// let diff = "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new";
/// let formatted = format_diff_for_terminal(diff);
/// print!("{}", formatted);
/// ```
pub fn format_diff_for_terminal(diff: &str) -> String {
    highlight_diff(diff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_diff_colors_reach_the_output() {
        let colors = crate::diff_theme::DiffColors {
            add_fg: termimad::crossterm::style::Color::Blue,
            ..Default::default()
        };
        let result = highlight_with_basic_colors("+added", &colors);
        assert!(result.contains("\x1b[94m"));
        assert!(!result.contains("\x1b[32m"));
    }

    #[test]
    fn test_basic_colors_addition() {
        let diff = "+added line";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[32m")); // Green
        assert!(result.contains("added line"));
        assert!(result.contains("\x1b[0m")); // Reset
    }

    #[test]
    fn test_basic_colors_deletion() {
        let diff = "-removed line";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[31m")); // Red
        assert!(result.contains("removed line"));
        assert!(result.contains("\x1b[0m")); // Reset
    }

    #[test]
    fn test_basic_colors_hunk_header() {
        let diff = "@@ -1,3 +1,4 @@";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[36m")); // Cyan
        assert!(result.contains("@@ -1,3 +1,4 @@"));
    }

    #[test]
    fn test_basic_colors_file_headers() {
        let diff = "--- a/file.rs\n+++ b/file.rs";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[1m")); // Bold
        assert!(result.contains("--- a/file.rs"));
        assert!(result.contains("+++ b/file.rs"));
    }

    #[test]
    fn test_basic_colors_diff_header() {
        let diff = "diff --git a/file.rs b/file.rs";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[1m")); // Bold
        assert!(result.contains("diff --git"));
    }

    #[test]
    fn test_basic_colors_index_line() {
        let diff = "index abc123..def456 100644";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[1m")); // Bold
    }

    #[test]
    fn test_basic_colors_context_line() {
        let diff = " context line";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        // Context lines should not have color codes (just the line + newline)
        assert!(!result.contains("\x1b[31m")); // No red
        assert!(!result.contains("\x1b[32m")); // No green
        assert!(result.contains("context line"));
    }

    #[test]
    fn test_basic_colors_full_diff() {
        let diff = r#"diff --git a/src/main.rs b/src/main.rs
index abc123..def456 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@
 fn main() {
+    println!("Hello");
-    println!("World");
 }
"#;
        let result = highlight_with_basic_colors(diff, &DiffColors::default());

        // Should have multiple color codes
        assert!(result.contains("\x1b[1m")); // Bold for headers
        assert!(result.contains("\x1b[32m")); // Green for additions
        assert!(result.contains("\x1b[31m")); // Red for deletions
        assert!(result.contains("\x1b[36m")); // Cyan for hunk header
    }

    #[test]
    fn test_basic_colors_empty_diff() {
        let result = highlight_with_basic_colors("", &DiffColors::default());
        assert!(result.is_empty());
    }

    #[test]
    fn test_basic_colors_no_changes() {
        let diff = " line 1\n line 2\n line 3";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        // No color codes for context-only diff
        assert!(!result.contains("\x1b[31m"));
        assert!(!result.contains("\x1b[32m"));
    }

    #[test]
    fn test_diff_highlighter_new() {
        let highlighter = DiffHighlighter::new();
        // Should not panic, delta availability is determined
        let _ = highlighter.is_delta_available();
    }

    #[test]
    fn test_diff_highlighter_default() {
        let highlighter = DiffHighlighter::default();
        let _ = highlighter.is_delta_available();
    }

    #[test]
    fn test_diff_highlighter_highlight_always() {
        let highlighter = DiffHighlighter::new();
        let diff = "+added\n-removed";
        let result = highlighter.highlight_always(diff);

        // Should always have highlighting (delta or basic)
        // Basic coloring adds green for + lines and red for - lines
        assert!(result.contains("added"));
        assert!(result.contains("removed"));
        // The result will have ANSI codes if basic coloring was applied
        // (delta might produce different output, but basic always produces ANSI)
        // Let's just verify the content is there since terminal detection varies
    }

    #[test]
    fn test_basic_colors_always_produces_ansi() {
        // This test verifies basic coloring always produces ANSI codes
        let diff = "+added\n-removed";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("\x1b[")); // Always has ANSI codes
    }

    #[test]
    fn test_basic_colors_preserves_content() {
        let diff = "+new line\n-old line\n context";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());

        // After stripping ANSI codes, content should be preserved
        let stripped = strip_ansi_codes(&result);
        assert!(stripped.contains("new line"));
        assert!(stripped.contains("old line"));
        assert!(stripped.contains("context"));
    }

    #[test]
    fn test_basic_colors_special_characters() {
        let diff = "+line with special: <>&\"'$`";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("<>&\"'$`"));
    }

    #[test]
    fn test_basic_colors_unicode() {
        let diff = "+Unicode: 世界 🎉 αβγ";
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        assert!(result.contains("世界"));
        assert!(result.contains("🎉"));
        assert!(result.contains("αβγ"));
    }

    #[test]
    fn test_basic_colors_multiple_hunks() {
        let diff = r#"@@ -1,2 +1,2 @@
-old1
+new1
@@ -10,2 +10,2 @@
-old2
+new2
"#;
        let result = highlight_with_basic_colors(diff, &DiffColors::default());
        // Should have two hunk headers colored
        assert_eq!(result.matches("\x1b[36m").count(), 2);
    }

    #[test]
    fn test_is_delta_available_does_not_crash() {
        // This test just verifies the function doesn't panic
        let _ = is_delta_available();
    }

    // Helper function to strip ANSI escape codes for testing
    fn strip_ansi_codes(s: &str) -> String {
        let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        re.replace_all(s, "").to_string()
    }

    #[test]
    fn every_diff_line_kind_uses_its_own_color() {
        let diff = "+a\n-b\n@@ -1 +1 @@";
        let result = highlight_with_basic_colors(diff, &crate::diff_theme::test_colors());
        assert!(result.contains("\x1b[94m+a"));
        assert!(result.contains("\x1b[95m-b"));
        assert!(result.contains("\x1b[93m@@ -1 +1 @@"));
    }
}
