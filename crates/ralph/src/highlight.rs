//! Syntax highlighting for code chunks (Imperative Shell).
//!
//! This module provides syntax highlighting functionality for code chunks
//! using the syntect library. It follows the Imperative Shell pattern,
//! handling terminal output with ANSI escape codes.
//!
//! # Features
//!
//! - Syntax highlighting for common programming languages
//! - Terminal detection for automatic color support
//! - Graceful fallback to plain text when not supported
//! - ANSI 24-bit true color output
//!
//! # Example
//!
//! ```no_run
//! use ralph::highlight::{highlight_code, is_highlighting_supported};
//!
//! if is_highlighting_supported() {
//!     let highlighted = highlight_code("fn main() {}", Some("rust"));
//!     print!("{}", highlighted);
//! } else {
//!     print!("fn main() {{}}");
//! }
//! ```

use std::io::IsTerminal;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};

/// A code highlighter using syntect.
///
/// This struct lazily loads syntax definitions and themes on first use,
/// caching them for subsequent highlighting operations.
pub struct Highlighter {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    /// Create a new highlighter with default syntax definitions and themes.
    ///
    /// This loads the bundled syntect defaults which include support for
    /// many common languages like Rust, Python, JavaScript, Go, etc.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let highlighter = Highlighter::new();
    /// ```
    pub fn new() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    /// Highlight code with optional language hint.
    ///
    /// Returns the highlighted code as a string with ANSI escape sequences
    /// for terminal output. If the language is unknown or highlighting fails,
    /// returns the original code unchanged.
    ///
    /// # Arguments
    ///
    /// * `code` - The source code to highlight
    /// * `language` - Optional language hint (e.g., "rust", "python", "js")
    ///
    /// # Returns
    ///
    /// A string containing the code with ANSI color codes.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let highlighter = Highlighter::new();
    /// let highlighted = highlighter.highlight("fn main() {}", Some("rust"));
    /// // highlighted contains ANSI escape sequences
    /// ```
    pub fn highlight(&self, code: &str, language: Option<&str>) -> String {
        // Try to find syntax for the language
        let syntax = match language {
            Some(lang) => self
                .syntax_set
                .find_syntax_by_token(lang)
                .or_else(|| self.syntax_set.find_syntax_by_extension(lang)),
            None => None,
        };

        // If no syntax found, return code as-is
        let syntax = match syntax {
            Some(s) => s,
            None => return code.to_string(),
        };

        // Use a dark theme suitable for terminals (base16-ocean.dark)
        let theme = &self.theme_set.themes["base16-ocean.dark"];

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut output = String::new();

        for line in LinesWithEndings::from(code) {
            match highlighter.highlight_line(line, &self.syntax_set) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges[..], true);
                    output.push_str(&escaped);
                }
                Err(_) => {
                    // If highlighting fails for a line, include it as-is
                    output.push_str(line);
                }
            }
        }

        // Reset terminal colors at the end
        output.push_str("\x1b[0m");

        output
    }

    /// Get a list of supported language tokens.
    ///
    /// Returns the primary file extensions/tokens for all loaded syntaxes.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let highlighter = Highlighter::new();
    /// let languages = highlighter.supported_languages();
    /// assert!(languages.contains(&"rs"));
    /// assert!(languages.contains(&"py"));
    /// ```
    pub fn supported_languages(&self) -> Vec<&str> {
        self.syntax_set
            .syntaxes()
            .iter()
            .flat_map(|s| s.file_extensions.iter().map(|e| e.as_str()))
            .collect()
    }

    /// Check if a language is supported for highlighting.
    ///
    /// # Arguments
    ///
    /// * `language` - The language token to check (e.g., "rust", "py", "js")
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let highlighter = Highlighter::new();
    /// assert!(highlighter.is_language_supported("rust"));
    /// assert!(highlighter.is_language_supported("python"));
    /// assert!(!highlighter.is_language_supported("made_up_language"));
    /// ```
    pub fn is_language_supported(&self, language: &str) -> bool {
        self.syntax_set
            .find_syntax_by_token(language)
            .or_else(|| self.syntax_set.find_syntax_by_extension(language))
            .is_some()
    }
}

/// Check if syntax highlighting is supported in the current environment.
///
/// Returns `true` if stdout is connected to a terminal that supports
/// ANSI escape codes. Returns `false` if output is piped or redirected.
///
/// # Example
///
/// ```no_run
/// use ralph::highlight::is_highlighting_supported;
///
/// if is_highlighting_supported() {
///     // Use colors and highlighting
/// } else {
///     // Plain text output
/// }
/// ```
pub fn is_highlighting_supported() -> bool {
    std::io::stdout().is_terminal()
}

/// Highlight code with the default highlighter.
///
/// This is a convenience function that creates a highlighter and highlights
/// the given code. For multiple highlighting operations, prefer creating
/// a [`Highlighter`] instance and reusing it.
///
/// # Arguments
///
/// * `code` - The source code to highlight
/// * `language` - Optional language hint (e.g., "rust", "python", "js")
///
/// # Returns
///
/// A string containing the code with ANSI color codes if a language was
/// recognized, or the original code unchanged if not.
///
/// # Example
///
/// ```
/// use ralph::highlight::highlight_code;
///
/// let rust_code = "fn main() { println!(\"Hello\"); }";
/// let highlighted = highlight_code(rust_code, Some("rust"));
/// ```
pub fn highlight_code(code: &str, language: Option<&str>) -> String {
    let highlighter = Highlighter::new();
    highlighter.highlight(code, language)
}

/// Format a code chunk for terminal output.
///
/// If terminal highlighting is supported and a language is provided,
/// returns highlighted code. Otherwise returns the code as-is.
///
/// # Arguments
///
/// * `code` - The source code to format
/// * `language` - Optional language hint
///
/// # Example
///
/// ```no_run
/// use ralph::highlight::format_code_for_terminal;
///
/// let formatted = format_code_for_terminal("print('hello')", Some("python"));
/// print!("{}", formatted);
/// ```
pub fn format_code_for_terminal(code: &str, language: Option<&str>) -> String {
    if is_highlighting_supported() && language.is_some() {
        highlight_code(code, language)
    } else {
        code.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlighter_new() {
        let highlighter = Highlighter::new();
        // Should have loaded some syntaxes
        assert!(!highlighter.supported_languages().is_empty());
    }

    #[test]
    fn test_highlighter_default() {
        let highlighter = Highlighter::default();
        assert!(!highlighter.supported_languages().is_empty());
    }

    #[test]
    fn test_highlight_rust_code() {
        let highlighter = Highlighter::new();
        let code = "fn main() { println!(\"Hello\"); }";
        let highlighted = highlighter.highlight(code, Some("rust"));

        // Should contain ANSI escape codes
        assert!(highlighted.contains("\x1b["));
        // Should contain the code content
        assert!(highlighted.contains("fn"));
        assert!(highlighted.contains("main"));
        // Should end with reset sequence
        assert!(highlighted.ends_with("\x1b[0m"));
    }

    #[test]
    fn test_highlight_python_code() {
        let highlighter = Highlighter::new();
        let code = "def hello():\n    print('hello')";
        let highlighted = highlighter.highlight(code, Some("python"));

        assert!(highlighted.contains("\x1b["));
        assert!(highlighted.contains("def"));
        assert!(highlighted.contains("hello"));
    }

    #[test]
    fn test_highlight_javascript_code() {
        let highlighter = Highlighter::new();
        let code = "const foo = () => console.log('bar');";
        let highlighted = highlighter.highlight(code, Some("js"));

        assert!(highlighted.contains("\x1b["));
        assert!(highlighted.contains("const"));
    }

    #[test]
    fn test_highlight_unknown_language() {
        let highlighter = Highlighter::new();
        let code = "some code here";
        let highlighted = highlighter.highlight(code, Some("not_a_real_language"));

        // Should return code unchanged (no highlighting)
        assert_eq!(highlighted, code);
    }

    #[test]
    fn test_highlight_no_language() {
        let highlighter = Highlighter::new();
        let code = "some code here";
        let highlighted = highlighter.highlight(code, None);

        // Should return code unchanged
        assert_eq!(highlighted, code);
    }

    #[test]
    fn test_highlight_empty_code() {
        let highlighter = Highlighter::new();
        let highlighted = highlighter.highlight("", Some("rust"));

        // Should just have reset sequence or be empty
        assert!(highlighted.is_empty() || highlighted == "\x1b[0m");
    }

    #[test]
    fn test_highlight_multiline_code() {
        let highlighter = Highlighter::new();
        let code = "fn main() {\n    println!(\"Hello\");\n}";
        let highlighted = highlighter.highlight(code, Some("rust"));

        // Should contain newlines
        assert!(highlighted.contains('\n'));
        // Should have ANSI codes
        assert!(highlighted.contains("\x1b["));
    }

    #[test]
    fn test_is_language_supported_rust() {
        let highlighter = Highlighter::new();
        assert!(highlighter.is_language_supported("rust"));
        assert!(highlighter.is_language_supported("rs"));
    }

    #[test]
    fn test_is_language_supported_python() {
        let highlighter = Highlighter::new();
        assert!(highlighter.is_language_supported("python"));
        assert!(highlighter.is_language_supported("py"));
    }

    #[test]
    fn test_is_language_supported_javascript() {
        let highlighter = Highlighter::new();
        assert!(highlighter.is_language_supported("javascript"));
        assert!(highlighter.is_language_supported("js"));
    }

    #[test]
    fn test_is_language_supported_common_languages() {
        let highlighter = Highlighter::new();

        // Common languages that syntect's defaults include
        let common = [
            "go", "java", "c", "cpp", "ruby", "sh", "json", "yaml", "sql",
        ];

        for lang in &common {
            assert!(
                highlighter.is_language_supported(lang),
                "{} should be supported",
                lang
            );
        }
    }

    #[test]
    fn test_is_language_supported_unknown() {
        let highlighter = Highlighter::new();
        assert!(!highlighter.is_language_supported("not_a_language"));
        assert!(!highlighter.is_language_supported("fake_lang_xyz"));
    }

    #[test]
    fn test_highlight_code_convenience_function() {
        let code = "fn test() {}";
        let highlighted = highlight_code(code, Some("rust"));

        // Should have highlighting
        assert!(highlighted.contains("\x1b["));
    }

    #[test]
    fn test_highlight_code_no_language() {
        let code = "some text";
        let highlighted = highlight_code(code, None);

        // Should return unchanged
        assert_eq!(highlighted, code);
    }

    #[test]
    fn test_highlight_preserves_content() {
        let highlighter = Highlighter::new();
        let code = "fn main() {\n    let x = 42;\n    println!(\"{}\", x);\n}";
        let highlighted = highlighter.highlight(code, Some("rust"));

        // After stripping ANSI codes, content should be preserved
        let stripped = strip_ansi_codes(&highlighted);
        assert_eq!(stripped.trim(), code);
    }

    #[test]
    fn test_highlight_special_characters() {
        let highlighter = Highlighter::new();
        // Code with special characters
        let code = "let s = \"Hello, 世界! <>&\";";
        let highlighted = highlighter.highlight(code, Some("rust"));

        // Should contain the special characters
        assert!(highlighted.contains("世界"));
        assert!(highlighted.contains("<>&"));
    }

    #[test]
    fn test_supported_languages_includes_common() {
        let highlighter = Highlighter::new();
        let languages = highlighter.supported_languages();

        // Should include common file extensions
        assert!(languages.contains(&"rs"), "Should support .rs files");
        assert!(languages.contains(&"py"), "Should support .py files");
        assert!(languages.contains(&"js"), "Should support .js files");
    }

    // Helper function to strip ANSI escape codes for testing
    fn strip_ansi_codes(s: &str) -> String {
        let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        re.replace_all(s, "").to_string()
    }
}
