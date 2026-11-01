use crate::diff_theme::DiffColors;

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
