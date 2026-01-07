//! Syntax highlighting for code chunks (Imperative Shell).
//!
//! This module provides syntax highlighting functionality for code chunks
//! using the syntect library. It follows the Imperative Shell pattern,
//! handling terminal output with ANSI escape codes.
//!
//! # Features
//!
//! - Syntax highlighting for common programming languages
//! - Configurable themes (built-in and custom .tmTheme files)
//! - Optional background color control
//! - Terminal detection for automatic color support
//! - Graceful fallback to plain text when not supported
//! - ANSI 24-bit true color output
//!
//! # Example
//!
//! ```no_run
//! use ralph::highlight::{highlight_code, is_highlighting_supported, ThemeConfig};
//!
//! if is_highlighting_supported() {
//!     let highlighted = highlight_code("fn main() {}", Some("rust"));
//!     print!("{}", highlighted);
//! } else {
//!     print!("fn main() {{}}");
//! }
//!
//! // With custom theme configuration
//! use ralph::highlight::Highlighter;
//! let config = ThemeConfig::new().with_theme("Monokai Extended");
//! let highlighter = Highlighter::with_config(config);
//! let highlighted = highlighter.highlight("fn main() {}", Some("rust"));
//! ```

use std::io::IsTerminal;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::{as_24_bit_terminal_escaped, LinesWithEndings};

/// Default theme name used when no theme is specified.
pub const DEFAULT_THEME: &str = "base16-ocean.dark";

/// Environment variable for theme selection.
pub const RALPH_THEME_ENV: &str = "RALPH_THEME";

/// Environment variable for disabling background colors.
pub const RALPH_NO_BACKGROUND_ENV: &str = "RALPH_NO_BACKGROUND";

/// Error type for theme operations.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    /// The requested theme was not found in the available themes.
    #[error("Theme '{name}' not found. Available themes: {available}")]
    ThemeNotFound { name: String, available: String },

    /// Failed to load a theme file from disk.
    #[error("Failed to load theme file '{path}': {source}")]
    LoadThemeFile {
        path: String,
        #[source]
        source: syntect::LoadingError,
    },
}

/// Configuration for syntax highlighting themes.
///
/// This struct provides a builder pattern for configuring the theme used
/// by the highlighter. It supports:
/// - Built-in syntect themes (e.g., "base16-ocean.dark", "Monokai Extended")
/// - Custom .tmTheme files from disk
/// - Controlling whether background colors are applied
///
/// # Example
///
/// ```
/// use ralph::highlight::ThemeConfig;
///
/// // Use default theme
/// let config = ThemeConfig::new();
///
/// // Use a specific built-in theme
/// let config = ThemeConfig::new().with_theme("Monokai Extended");
///
/// // Load a custom theme file
/// let config = ThemeConfig::new().with_theme_file("/path/to/theme.tmTheme");
///
/// // Disable background colors
/// let config = ThemeConfig::new().with_no_background(true);
/// ```
#[derive(Debug, Clone, Default)]
pub struct ThemeConfig {
    /// Theme name or path. None uses the default.
    theme: Option<String>,
    /// Whether to disable background colors.
    no_background: bool,
}

impl ThemeConfig {
    /// Create a new theme configuration with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a theme configuration from environment variables only.
    ///
    /// Reads from:
    /// - `RALPH_THEME` - Theme name or file path
    /// - `RALPH_NO_BACKGROUND` - Set to any non-empty value to disable backgrounds
    ///
    /// Note: For full configuration precedence (config file + env), use `from_config_and_env()`.
    pub fn from_env() -> Self {
        let theme = std::env::var(RALPH_THEME_ENV).ok();
        let no_background = std::env::var(RALPH_NO_BACKGROUND_ENV)
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        Self {
            theme,
            no_background,
        }
    }

    /// Create a theme configuration from config file and environment variables.
    ///
    /// Loads configuration in this order (higher priority sources override lower):
    /// 1. Environment variables (`RALPH_THEME`, `RALPH_NO_BACKGROUND`)
    /// 2. Config file (`~/.config/ralph/config.toml`)
    /// 3. Default values
    ///
    /// If the config file doesn't exist or fails to parse, falls back to env + defaults.
    pub fn from_config_and_env() -> Self {
        // Start with config file settings (if available)
        let config_theme = crate::config::AppConfig::load()
            .ok()
            .and_then(|c| c.theme.name);
        let config_no_background = crate::config::AppConfig::load()
            .ok()
            .map(|c| c.theme.no_background)
            .unwrap_or(false);

        // Get environment variable values
        let env_theme = std::env::var(RALPH_THEME_ENV).ok();
        let env_no_background = std::env::var(RALPH_NO_BACKGROUND_ENV)
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        // Env vars take precedence over config file
        let theme = env_theme.or(config_theme);
        let no_background = env_no_background || config_no_background;

        Self {
            theme,
            no_background,
        }
    }

    /// Set the theme name or file path.
    ///
    /// If the value looks like a file path (contains `/` or `\`, or ends in `.tmTheme`),
    /// it will be treated as a custom theme file. Otherwise, it's treated as a
    /// built-in theme name.
    pub fn with_theme(mut self, theme: impl Into<String>) -> Self {
        self.theme = Some(theme.into());
        self
    }

    /// Set the theme from a file path.
    ///
    /// The file should be a valid TextMate .tmTheme file.
    pub fn with_theme_file(self, path: impl Into<String>) -> Self {
        self.with_theme(path)
    }

    /// Set whether to disable background colors.
    ///
    /// When enabled, the theme's background colors are not applied,
    /// allowing the terminal's default background to show through.
    pub fn with_no_background(mut self, no_background: bool) -> Self {
        self.no_background = no_background;
        self
    }

    /// Get the theme name or path, if set.
    pub fn theme(&self) -> Option<&str> {
        self.theme.as_deref()
    }

    /// Check if background colors should be disabled.
    pub fn no_background(&self) -> bool {
        self.no_background
    }

    /// Check if the theme looks like a file path.
    pub fn is_theme_file(&self) -> bool {
        self.theme
            .as_ref()
            .map(|t| t.contains('/') || t.contains('\\') || t.ends_with(".tmTheme"))
            .unwrap_or(false)
    }

    /// Merge with CLI options, where CLI takes precedence.
    ///
    /// This allows layering: config file -> env var -> CLI flag
    pub fn merge_cli(mut self, theme: Option<&str>, no_background: bool) -> Self {
        if let Some(t) = theme {
            self.theme = Some(t.to_string());
        }
        if no_background {
            self.no_background = true;
        }
        self
    }
}

/// A code highlighter using syntect.
///
/// This struct lazily loads syntax definitions and themes on first use,
/// caching them for subsequent highlighting operations. It supports
/// configurable themes including built-in themes and custom .tmTheme files.
#[derive(Debug)]
pub struct Highlighter {
    syntax_set: SyntaxSet,
    /// The resolved theme to use for highlighting.
    theme: Theme,
    /// Whether to include background colors in output.
    use_background: bool,
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
    /// Uses the default theme (base16-ocean.dark).
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let highlighter = Highlighter::new();
    /// ```
    pub fn new() -> Self {
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set.themes[DEFAULT_THEME].clone();
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme,
            use_background: true,
        }
    }

    /// Create a highlighter with custom theme configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - Theme configuration specifying theme name/path and background option
    ///
    /// # Returns
    ///
    /// * `Ok(Highlighter)` - Successfully configured highlighter
    /// * `Err(ThemeError)` - If the theme was not found or failed to load
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::{Highlighter, ThemeConfig};
    ///
    /// let config = ThemeConfig::new().with_theme("Monokai Extended");
    /// let highlighter = Highlighter::with_config(config).unwrap();
    /// ```
    pub fn with_config(config: ThemeConfig) -> Result<Self, ThemeError> {
        let theme_set = ThemeSet::load_defaults();
        let syntax_set = SyntaxSet::load_defaults_newlines();

        let theme = match config.theme() {
            Some(theme_name) if config.is_theme_file() => {
                // Load from file
                ThemeSet::get_theme(theme_name).map_err(|e| ThemeError::LoadThemeFile {
                    path: theme_name.to_string(),
                    source: e,
                })?
            }
            Some(theme_name) => {
                // Look up built-in theme
                theme_set.themes.get(theme_name).cloned().ok_or_else(|| {
                    ThemeError::ThemeNotFound {
                        name: theme_name.to_string(),
                        available: Self::format_available_themes(&theme_set),
                    }
                })?
            }
            None => theme_set.themes[DEFAULT_THEME].clone(),
        };

        Ok(Self {
            syntax_set,
            theme,
            use_background: !config.no_background(),
        })
    }

    /// Create a highlighter with a specific theme name.
    ///
    /// This is a convenience method for creating a highlighter with just a theme name.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let highlighter = Highlighter::with_theme("Monokai Extended").unwrap();
    /// ```
    pub fn with_theme(theme_name: &str) -> Result<Self, ThemeError> {
        Self::with_config(ThemeConfig::new().with_theme(theme_name))
    }

    /// Get a list of available built-in theme names.
    ///
    /// # Example
    ///
    /// ```
    /// use ralph::highlight::Highlighter;
    ///
    /// let themes = Highlighter::available_themes();
    /// assert!(themes.contains(&"base16-ocean.dark".to_string()));
    /// ```
    pub fn available_themes() -> Vec<String> {
        let theme_set = ThemeSet::load_defaults();
        let mut themes: Vec<_> = theme_set.themes.keys().cloned().collect();
        themes.sort();
        themes
    }

    /// Format available themes as a comma-separated string for error messages.
    fn format_available_themes(theme_set: &ThemeSet) -> String {
        let mut themes: Vec<_> = theme_set.themes.keys().collect();
        themes.sort();
        themes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
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

        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        let mut output = String::new();

        for line in LinesWithEndings::from(code) {
            match highlighter.highlight_line(line, &self.syntax_set) {
                Ok(ranges) => {
                    let escaped = as_24_bit_terminal_escaped(&ranges[..], self.use_background);
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

    #[test]
    fn test_theme_config_new() {
        let config = ThemeConfig::new();
        assert!(config.theme().is_none());
        assert!(!config.no_background());
    }

    #[test]
    fn test_theme_config_with_theme() {
        let config = ThemeConfig::new().with_theme("Monokai Extended");
        assert_eq!(config.theme(), Some("Monokai Extended"));
    }

    #[test]
    fn test_theme_config_with_no_background() {
        let config = ThemeConfig::new().with_no_background(true);
        assert!(config.no_background());
    }

    #[test]
    fn test_theme_config_is_theme_file_with_path() {
        let config = ThemeConfig::new().with_theme("/path/to/theme.tmTheme");
        assert!(config.is_theme_file());
    }

    #[test]
    fn test_theme_config_is_theme_file_with_name() {
        let config = ThemeConfig::new().with_theme("Monokai Extended");
        assert!(!config.is_theme_file());
    }

    #[test]
    fn test_theme_config_is_theme_file_with_tmtheme_extension() {
        let config = ThemeConfig::new().with_theme("custom.tmTheme");
        assert!(config.is_theme_file());
    }

    #[test]
    fn test_theme_config_merge_cli_overrides_theme() {
        let config = ThemeConfig::new()
            .with_theme("base16-ocean.dark")
            .merge_cli(Some("Monokai Extended"), false);

        assert_eq!(config.theme(), Some("Monokai Extended"));
    }

    #[test]
    fn test_theme_config_merge_cli_preserves_theme_when_none() {
        let config = ThemeConfig::new()
            .with_theme("base16-ocean.dark")
            .merge_cli(None, false);

        assert_eq!(config.theme(), Some("base16-ocean.dark"));
    }

    #[test]
    fn test_theme_config_merge_cli_overrides_no_background() {
        let config = ThemeConfig::new()
            .with_no_background(false)
            .merge_cli(None, true);

        assert!(config.no_background());
    }

    #[test]
    fn test_theme_config_merge_cli_preserves_no_background_when_false() {
        let config = ThemeConfig::new()
            .with_no_background(true)
            .merge_cli(None, false);

        // merge_cli only overrides if the CLI value is true
        assert!(config.no_background());
    }

    #[test]
    fn test_highlighter_with_config_valid_theme() {
        let config = ThemeConfig::new().with_theme("base16-ocean.dark");
        let highlighter = Highlighter::with_config(config);
        assert!(highlighter.is_ok());
    }

    #[test]
    fn test_highlighter_with_config_invalid_theme() {
        let config = ThemeConfig::new().with_theme("nonexistent-theme");
        let result = Highlighter::with_config(config);
        assert!(result.is_err());

        // Verify the error message contains expected text
        let err_str = result.unwrap_err().to_string();
        assert!(err_str.contains("nonexistent-theme"));
        assert!(err_str.contains("not found"));
    }

    #[test]
    fn test_highlighter_with_config_no_background() {
        let config = ThemeConfig::new().with_no_background(true);
        let highlighter = Highlighter::with_config(config).unwrap();

        // Verify highlighting still works
        let highlighted = highlighter.highlight("fn test() {}", Some("rust"));
        assert!(highlighted.contains("\x1b["));
    }

    #[test]
    fn test_available_themes_not_empty() {
        let themes = Highlighter::available_themes();
        assert!(!themes.is_empty());
        assert!(themes.contains(&DEFAULT_THEME.to_string()));
    }
}
