//! Pure utility functions for output rendering (Functional Core).
//!
//! This module contains stateless helper functions used by both stream processor
//! and replay renderer. All functions are pure with no side effects.

use std::collections::BTreeMap;

/// Extract line number from a cat-n formatted line, if present.
///
/// Returns `(line_number_str, rest_of_line)` or `None` if not a cat-n line.
///
/// # Format Support
///
/// Handles both tab (`\t`) and arrow (`→`) separators as Claude CLI may use either:
/// - `     1\tcontent` → Some(("1", "content"))
/// - `    12→content` → Some(("12", "content"))
///
/// # Examples
///
/// ```
/// use ralph::render::extract_line_number;
///
/// assert_eq!(extract_line_number("     1\tfn main()"), Some(("1", "fn main()")));
/// assert_eq!(extract_line_number("    12→  let x = 1;"), Some(("12", "  let x = 1;")));
/// assert_eq!(extract_line_number("plain text"), None);
/// ```
pub fn extract_line_number(line: &str) -> Option<(&str, &str)> {
    // Try to find separator: tab or arrow character
    let separator_pos = line.find('\t').or_else(|| line.find('→'));

    if let Some(sep_pos) = separator_pos {
        let prefix = &line[..sep_pos];
        // Arrow is multi-byte UTF-8, so we need to handle it properly
        let rest = if line[sep_pos..].starts_with('→') {
            &line[sep_pos + '→'.len_utf8()..]
        } else {
            &line[sep_pos + 1..] // tab is single byte
        };

        // Check if prefix is whitespace followed by digits
        let trimmed = prefix.trim_start();
        if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
            return Some((trimmed, rest));
        }
    }
    None
}

/// Normalize Claude CLI's `cat -n` line number format to a cleaner pipe-separated format.
///
/// Transforms lines from:
/// - `     1\tcontent` → ` 1 │ content`
/// - `    12→content` → `12 │ content`
/// - `   123\tcontent` → `123 │ content`
///
/// Handles both tab (`\t`) and arrow (`→`) separators as Claude CLI may use either.
/// Line numbers are right-aligned to the width of the largest line number.
/// Lines that don't match the pattern are passed through unchanged.
///
/// # Examples
///
/// ```
/// use ralph::render::normalize_cat_n_format;
///
/// let input = "     1\tfn main() {\n     2\t    println!(\"hello\");\n     3\t}";
/// let expected = "1 │ fn main() {\n2 │     println!(\"hello\");\n3 │ }";
/// assert_eq!(normalize_cat_n_format(input), expected);
/// ```
pub fn normalize_cat_n_format(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    // First pass: find max line number width
    let max_width = lines
        .iter()
        .filter_map(|line| extract_line_number(line))
        .map(|(num, _)| num.len())
        .max()
        .unwrap_or(1);

    // Second pass: format with consistent width
    lines
        .iter()
        .map(|line| {
            if let Some((num, rest)) = extract_line_number(line) {
                format!("{:>width$} │ {}", num, rest, width = max_width)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Group file paths by their parent directory.
///
/// Returns a sorted map of directory -> list of full file paths.
/// Files in the root (no directory) are grouped under an empty string key.
///
/// # Examples
///
/// ```
/// use ralph::render::group_files_by_directory;
///
/// let files = vec!["src/main.rs", "src/lib.rs", "Cargo.toml"];
/// let grouped = group_files_by_directory(&files);
///
/// assert_eq!(grouped.get("src").map(|v| v.len()), Some(2));
/// assert_eq!(grouped.get("").map(|v| v.len()), Some(1)); // root files
/// ```
pub fn group_files_by_directory<'a>(files: &[&'a str]) -> BTreeMap<String, Vec<&'a str>> {
    let mut grouped: BTreeMap<String, Vec<&'a str>> = BTreeMap::new();

    for file in files {
        let dir = match file.rfind('/') {
            Some(pos) => file[..pos].to_string(),
            None => String::new(), // No directory, use empty string for root
        };
        grouped.entry(dir).or_default().push(file);
    }

    grouped
}

/// Highlight a grep match within a line of output.
///
/// Attempts to find and highlight the matched portion of the line.
/// For content mode output (`filename:line_number:content`), this highlights
/// the content portion where the pattern matched.
///
/// # Format
///
/// - `filename:line_number:content` → dim prefix, yellow content
/// - Other formats → dim entire line
///
/// # Examples
///
/// ```
/// use ralph::render::highlight_grep_match;
///
/// let line = "src/main.rs:10:fn main() {}";
/// let result = highlight_grep_match(line);
/// assert!(result.contains("\x1b[90m")); // dim
/// assert!(result.contains("\x1b[93m")); // yellow
/// ```
pub fn highlight_grep_match(line: &str) -> String {
    // Parse the line format: filename:line_number:content or just filename
    // We apply dim styling to the filename:line_number prefix
    // and yellow styling to the content

    // Try to find the pattern ":number:" which indicates content mode
    if let Some(first_colon) = line.find(':') {
        if let Some(second_colon_offset) = line[first_colon + 1..].find(':') {
            let second_colon = first_colon + 1 + second_colon_offset;
            // Check if the part between colons is a number
            let potential_line_num = &line[first_colon + 1..second_colon];
            if potential_line_num.chars().all(|c| c.is_ascii_digit()) {
                // This looks like filename:line_number:content format
                let prefix = &line[..second_colon + 1];
                let content = &line[second_colon + 1..];
                return format!("\x1b[90m{}\x1b[0m\x1b[93m{}\x1b[0m", prefix, content);
            }
        }
    }

    // Default: just show the line in dim color
    format!("\x1b[90m{}\x1b[0m", line)
}

/// Extract programming language hint from a file path based on extension.
///
/// Returns the language token that can be used with syntect for syntax highlighting.
/// Returns `None` for unknown extensions.
///
/// # Supported Languages
///
/// Covers 40+ file extensions including:
/// - Rust, Python, JavaScript/TypeScript, Go, Java, C/C++
/// - Ruby, PHP, Swift, Kotlin
/// - Shell scripts, Docker, Makefiles
/// - Data formats (JSON, YAML, TOML, XML)
/// - And many more
///
/// # Examples
///
/// ```
/// use ralph::render::extract_language_from_path;
///
/// assert_eq!(extract_language_from_path("main.rs"), Some("rust"));
/// assert_eq!(extract_language_from_path("config.yaml"), Some("yaml"));
/// assert_eq!(extract_language_from_path("unknown.xyz"), None);
/// ```
pub fn extract_language_from_path(file_path: &str) -> Option<&'static str> {
    // Get the extension from the file path
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())?;

    // Map common extensions to syntect language tokens
    match extension.to_lowercase().as_str() {
        // Rust
        "rs" => Some("rust"),
        // Python
        "py" | "pyw" | "pyi" => Some("python"),
        // JavaScript/TypeScript
        "js" | "mjs" | "cjs" => Some("javascript"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "jsx" => Some("jsx"),
        "tsx" => Some("tsx"),
        // Web
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "scss" | "sass" => Some("scss"),
        // Shell
        "sh" | "bash" | "zsh" => Some("sh"),
        // C/C++
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        // Go
        "go" => Some("go"),
        // Java/Kotlin
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        // Ruby
        "rb" => Some("ruby"),
        // PHP
        "php" => Some("php"),
        // Swift
        "swift" => Some("swift"),
        // Data formats
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        "xml" => Some("xml"),
        // Markup
        "md" | "markdown" => Some("markdown"),
        // SQL
        "sql" => Some("sql"),
        // Docker
        "dockerfile" => Some("dockerfile"),
        // Makefile
        "mk" | "makefile" => Some("makefile"),
        // Config
        "ini" | "cfg" => Some("ini"),
        // Diff
        "diff" | "patch" => Some("diff"),
        // Other
        "lua" => Some("lua"),
        "vim" => Some("viml"),
        "hs" => Some("haskell"),
        "ml" | "mli" => Some("ocaml"),
        "ex" | "exs" => Some("elixir"),
        "erl" | "hrl" => Some("erlang"),
        "clj" | "cljs" | "cljc" => Some("clojure"),
        "scala" | "sc" => Some("scala"),
        "r" => Some("r"),
        "pl" | "pm" => Some("perl"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // extract_line_number tests
    // =========================================================================

    #[test]
    fn test_extract_line_number_with_tab() {
        let (num, rest) = extract_line_number("     1\tfn main()").unwrap();
        assert_eq!(num, "1");
        assert_eq!(rest, "fn main()");
    }

    #[test]
    fn test_extract_line_number_with_arrow() {
        let (num, rest) = extract_line_number("      12→  let x = 1;").unwrap();
        assert_eq!(num, "12");
        assert_eq!(rest, "  let x = 1;");
    }

    #[test]
    fn test_extract_line_number_no_separator() {
        assert!(extract_line_number("plain text").is_none());
    }

    #[test]
    fn test_extract_line_number_non_numeric_prefix() {
        assert!(extract_line_number("abc\tcontent").is_none());
    }

    // =========================================================================
    // normalize_cat_n_format tests
    // =========================================================================

    #[test]
    fn test_normalize_cat_n_format_single_digit() {
        let input = "     1\tfn main() {";
        let expected = "1 │ fn main() {";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_double_digit() {
        let input = "    12\t    println!(\"hello\");";
        let expected = "12 │     println!(\"hello\");";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_triple_digit() {
        let input = "   123\t}";
        let expected = "123 │ }";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_multiple_lines() {
        let input = "     1\tfn main() {\n     2\t    println!(\"hello\");\n     3\t}";
        let expected = "1 │ fn main() {\n2 │     println!(\"hello\");\n3 │ }";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_pass_through_no_tab() {
        let input = "This is just plain text";
        assert_eq!(normalize_cat_n_format(input), input);
    }

    #[test]
    fn test_normalize_cat_n_format_pass_through_non_numeric_prefix() {
        let input = "abc\tcontent";
        assert_eq!(normalize_cat_n_format(input), input);
    }

    #[test]
    fn test_normalize_cat_n_format_preserves_content_with_tabs() {
        let input = "     1\tfield1\tfield2";
        let expected = "1 │ field1\tfield2";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_empty_content() {
        let input = "     1\t";
        let expected = "1 │ ";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_empty_string() {
        let input = "";
        assert_eq!(normalize_cat_n_format(input), "");
    }

    #[test]
    fn test_normalize_cat_n_format_mixed_lines() {
        let input = "     1\tcode line\nregular text\n     2\tmore code";
        let expected = "1 │ code line\nregular text\n2 │ more code";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_with_offset() {
        let input = "    50\tline fifty\n    51\tline fifty-one";
        let expected = "50 │ line fifty\n51 │ line fifty-one";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_arrow_single_digit() {
        let input = "       1→fn main() {";
        let expected = "1 │ fn main() {";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_arrow_double_digit() {
        let input = "      12→    println!(\"hello\");";
        let expected = "12 │     println!(\"hello\");";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_arrow_multiple_lines() {
        let input = "       1→fn main() {\n       2→    println!(\"hello\");\n       3→}";
        let expected = "1 │ fn main() {\n2 │     println!(\"hello\");\n3 │ }";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_arrow_with_offset() {
        let input = "      50→line fifty\n      51→line fifty-one";
        let expected = "50 │ line fifty\n51 │ line fifty-one";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_alignment_9_to_10() {
        let input = "       9→line nine\n      10→line ten";
        let expected = " 9 │ line nine\n10 │ line ten";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_alignment_99_to_100() {
        let input = "      99→line ninety-nine\n     100→line one hundred";
        let expected = " 99 │ line ninety-nine\n100 │ line one hundred";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    #[test]
    fn test_normalize_cat_n_format_alignment_mixed_widths() {
        let input = "       1→first\n      50→middle\n     100→last";
        let expected = "  1 │ first\n 50 │ middle\n100 │ last";
        assert_eq!(normalize_cat_n_format(input), expected);
    }

    // =========================================================================
    // group_files_by_directory tests
    // =========================================================================

    #[test]
    fn test_group_files_by_directory() {
        let files = vec![
            "src/main.rs",
            "src/lib.rs",
            "tests/integration.rs",
            "Cargo.toml",
        ];

        let grouped = group_files_by_directory(&files);

        assert_eq!(grouped.len(), 3);
        assert_eq!(grouped.get("src").map(|v| v.len()), Some(2));
        assert_eq!(grouped.get("tests").map(|v| v.len()), Some(1));
        assert_eq!(grouped.get("").map(|v| v.len()), Some(1)); // root files
    }

    #[test]
    fn test_group_files_nested_directories() {
        let files = vec![
            "src/stream_processor/mod.rs",
            "src/stream_processor/types.rs",
            "src/run/mod.rs",
        ];

        let grouped = group_files_by_directory(&files);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped.get("src/stream_processor").map(|v| v.len()),
            Some(2)
        );
        assert_eq!(grouped.get("src/run").map(|v| v.len()), Some(1));
    }

    #[test]
    fn test_group_files_empty_input() {
        let files: Vec<&str> = vec![];
        let grouped = group_files_by_directory(&files);
        assert!(grouped.is_empty());
    }

    // =========================================================================
    // highlight_grep_match tests
    // =========================================================================

    #[test]
    fn test_highlight_grep_match_content_format() {
        let line = "src/main.rs:10:fn main() {}";
        let result = highlight_grep_match(line);
        // Should contain ANSI codes for highlighting
        assert!(result.contains("\x1b[90m"));
        assert!(result.contains("\x1b[93m"));
        assert!(result.contains("src/main.rs:10:"));
        assert!(result.contains("fn main() {}"));
    }

    #[test]
    fn test_highlight_grep_match_simple_path() {
        let line = "src/main.rs";
        let result = highlight_grep_match(line);
        // Should be dimmed but not split
        assert!(result.contains("\x1b[90m"));
        assert!(result.contains("src/main.rs"));
    }

    #[test]
    fn test_highlight_grep_match_no_line_number() {
        let line = "filename:not_a_number:content";
        let result = highlight_grep_match(line);
        // Should be dimmed entirely (not split)
        assert_eq!(result, "\x1b[90mfilename:not_a_number:content\x1b[0m");
    }

    // =========================================================================
    // extract_language_from_path tests
    // =========================================================================

    #[test]
    fn test_extract_language_rust() {
        assert_eq!(extract_language_from_path("main.rs"), Some("rust"));
        assert_eq!(extract_language_from_path("src/lib.rs"), Some("rust"));
    }

    #[test]
    fn test_extract_language_python() {
        assert_eq!(extract_language_from_path("script.py"), Some("python"));
        assert_eq!(extract_language_from_path("types.pyi"), Some("python"));
    }

    #[test]
    fn test_extract_language_javascript_typescript() {
        assert_eq!(extract_language_from_path("app.js"), Some("javascript"));
        assert_eq!(extract_language_from_path("app.mjs"), Some("javascript"));
        assert_eq!(extract_language_from_path("app.ts"), Some("typescript"));
        assert_eq!(extract_language_from_path("Component.tsx"), Some("tsx"));
        assert_eq!(extract_language_from_path("Component.jsx"), Some("jsx"));
    }

    #[test]
    fn test_extract_language_data_formats() {
        assert_eq!(extract_language_from_path("config.json"), Some("json"));
        assert_eq!(extract_language_from_path("config.yaml"), Some("yaml"));
        assert_eq!(extract_language_from_path("config.yml"), Some("yaml"));
        assert_eq!(extract_language_from_path("config.toml"), Some("toml"));
    }

    #[test]
    fn test_extract_language_shell() {
        assert_eq!(extract_language_from_path("script.sh"), Some("sh"));
        assert_eq!(extract_language_from_path("script.bash"), Some("sh"));
        assert_eq!(extract_language_from_path("script.zsh"), Some("sh"));
    }

    #[test]
    fn test_extract_language_c_cpp() {
        assert_eq!(extract_language_from_path("main.c"), Some("c"));
        assert_eq!(extract_language_from_path("header.h"), Some("c"));
        assert_eq!(extract_language_from_path("main.cpp"), Some("cpp"));
        assert_eq!(extract_language_from_path("main.cxx"), Some("cpp"));
    }

    #[test]
    fn test_extract_language_unknown() {
        assert_eq!(extract_language_from_path("file.xyz"), None);
        assert_eq!(extract_language_from_path("no_extension"), None);
    }

    #[test]
    fn test_extract_language_case_insensitive() {
        assert_eq!(extract_language_from_path("main.RS"), Some("rust"));
        assert_eq!(extract_language_from_path("config.JSON"), Some("json"));
    }
}
