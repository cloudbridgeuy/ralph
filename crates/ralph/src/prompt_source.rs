//! Prompt source classification and reading (Functional Core + Imperative Shell).
//!
//! This module provides types and functions for resolving prompts from various sources:
//! - Stdin (when argument is "-")
//! - File path (when argument is an existing file)
//! - Inline string (when argument doesn't match a file)
//! - Default/none (when no argument provided)
//!
//! The classification is a pure function, while reading the content performs I/O.

use std::path::Path;

/// Input source for prompt resolution.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptSource<'a> {
    /// Read from stdin (when arg is "-")
    Stdin,
    /// Read from file at path
    File(&'a Path),
    /// Use inline string directly
    Inline(&'a str),
    /// No input provided
    None,
}

/// Classify the input argument into a source type.
///
/// This is a pure function that determines how to interpret the argument:
/// - "-" means stdin
/// - An existing file path means read from file
/// - Any other string is treated as inline content
/// - None means no input
pub fn classify_prompt_source(arg: Option<&str>) -> PromptSource<'_> {
    match arg {
        Some("-") => PromptSource::Stdin,
        Some(value) => {
            let path = Path::new(value);
            if path.exists() && path.is_file() {
                PromptSource::File(path)
            } else {
                PromptSource::Inline(value)
            }
        }
        None => PromptSource::None,
    }
}

/// Read content from a prompt source.
///
/// This is the imperative shell that performs actual I/O based on the source type.
///
/// # Arguments
///
/// * `source` - The classified prompt source
/// * `default` - Optional default content to use when source is `None`
///
/// # Returns
///
/// The content string from the specified source.
pub fn read_from_source(
    source: PromptSource<'_>,
    default: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    match source {
        PromptSource::Stdin => {
            use std::io::Read;
            let mut content = String::new();
            std::io::stdin().read_to_string(&mut content)?;
            Ok(content)
        }
        PromptSource::File(path) => Ok(std::fs::read_to_string(path)?),
        PromptSource::Inline(value) => Ok(value.to_string()),
        PromptSource::None => Ok(default.unwrap_or("").to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_prompt_source_stdin() {
        assert_eq!(classify_prompt_source(Some("-")), PromptSource::Stdin);
    }

    #[test]
    fn test_classify_prompt_source_none() {
        assert_eq!(classify_prompt_source(None), PromptSource::None);
    }

    #[test]
    fn test_classify_prompt_source_inline() {
        // Non-existent path should be treated as inline
        assert_eq!(
            classify_prompt_source(Some("inline content")),
            PromptSource::Inline("inline content")
        );
    }

    #[test]
    fn test_classify_prompt_source_file() {
        // Use Cargo.toml as a file that definitely exists
        let source = classify_prompt_source(Some("Cargo.toml"));
        assert!(matches!(source, PromptSource::File(_)));
    }

    #[test]
    fn test_read_from_source_inline() {
        let source = PromptSource::Inline("hello world");
        let result = read_from_source(source, None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn test_read_from_source_none_with_default() {
        let result = read_from_source(PromptSource::None, Some("default value"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "default value");
    }

    #[test]
    fn test_read_from_source_none_without_default() {
        let result = read_from_source(PromptSource::None, None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_read_from_source_file() {
        // Use Cargo.toml as a file that definitely exists
        let source = PromptSource::File(Path::new("Cargo.toml"));
        let result = read_from_source(source, None);
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.contains("[package]"));
    }
}
