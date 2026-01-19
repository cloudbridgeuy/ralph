//! Progress file auto-summarization.
//!
//! This module handles automatic summarization of the progress file when it
//! exceeds a configured line threshold. Following the Functional Core / Imperative
//! Shell pattern, pure functions handle content processing while I/O operations
//! are isolated in dedicated functions.

use crate::spinner::{Spinner, SpinnerContext};
use ralph_core::context::{defaults, substitute_summarize_placeholders};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Configuration for progress file summarization.
#[derive(Debug, Clone)]
pub struct SummarizeConfig {
    /// Maximum lines before triggering summarization. 0 means disabled.
    pub max_lines: usize,
    /// Command template to invoke (with {prompt} placeholder).
    pub command: String,
    /// Prompt template (with {progress_file}, {progress_content} placeholders).
    pub prompt: String,
    /// Whether summarization is disabled.
    pub disabled: bool,
}

impl Default for SummarizeConfig {
    fn default() -> Self {
        Self {
            max_lines: 1000,
            command: defaults::SUMMARIZE_COMMAND.to_string(),
            prompt: defaults::SUMMARIZE_PROMPT.to_string(),
            disabled: false,
        }
    }
}

impl SummarizeConfig {
    /// Check if summarization should be attempted based on config.
    pub fn should_summarize(&self) -> bool {
        !self.disabled && self.max_lines > 0
    }
}

/// Error type for summarization operations.
#[derive(thiserror::Error, Debug)]
pub enum SummarizeError {
    /// Failed to read progress file.
    #[error("Failed to read progress file: {0}")]
    ReadFile(#[source] std::io::Error),

    /// Failed to write progress file.
    #[error("Failed to write progress file: {0}")]
    WriteFile(#[source] std::io::Error),

    /// Failed to create temporary file.
    #[error("Failed to create temporary file: {0}")]
    CreateTempFile(#[source] std::io::Error),

    /// Failed to rename temporary file.
    #[error("Failed to rename temporary file: {0}")]
    RenameTempFile(#[source] std::io::Error),

    /// Failed to spawn summarization command.
    #[error("Failed to spawn summarization command: {0}")]
    SpawnCommand(#[source] std::io::Error),

    /// Summarization command failed.
    #[error("Summarization command failed with exit code {exit_code}: {stderr}")]
    CommandFailed { exit_code: i32, stderr: String },

    /// Summarization command timed out.
    #[error("Summarization command timed out after {timeout_secs} seconds")]
    Timeout { timeout_secs: u64 },

    /// Summarized content is empty.
    #[error("Summarization produced empty content - keeping original")]
    EmptyContent,
}

/// Result of checking if summarization is needed.
#[derive(Debug)]
pub struct SummarizeCheck {
    /// Whether summarization is needed.
    pub needed: bool,
    /// Current line count.
    pub line_count: usize,
    /// Threshold for triggering summarization.
    pub threshold: usize,
}

/// Count lines in the given content.
///
/// This is a pure function that counts lines in the given string.
/// Empty files return 0 lines.
pub fn count_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

/// Check if summarization is needed based on file content and config.
///
/// This is a pure function - actual file reading happens at the shell layer.
pub fn check_summarization_needed(content: &str, config: &SummarizeConfig) -> SummarizeCheck {
    let line_count = count_lines(content);
    let needed = config.should_summarize() && line_count > config.max_lines;

    SummarizeCheck {
        needed,
        line_count,
        threshold: config.max_lines,
    }
}

/// Build the final command string for summarization.
///
/// Substitutes placeholders in the prompt template, then in the command template.
pub fn build_summarize_command(
    config: &SummarizeConfig,
    progress_path: &str,
    progress_content: &str,
) -> String {
    // First substitute placeholders in the prompt
    let prompt = substitute_summarize_placeholders(&config.prompt, progress_path, progress_content);

    // Then substitute {prompt} in the command, with shell escaping
    let escaped = prompt.replace('\'', "'\"'\"'");
    let quoted_prompt = format!("'{}'", escaped);
    config.command.replace("{prompt}", &quoted_prompt)
}

/// Read progress file content.
///
/// This is the imperative shell function for reading the progress file.
pub fn read_progress_file(path: &Path) -> Result<String, SummarizeError> {
    std::fs::read_to_string(path).map_err(SummarizeError::ReadFile)
}

/// Write content to progress file atomically.
///
/// Uses a temporary file and rename to prevent corruption.
pub fn write_progress_file_atomic(path: &Path, content: &str) -> Result<(), SummarizeError> {
    // Create temp file in same directory to ensure rename works (same filesystem)
    let parent = path.parent().unwrap_or(Path::new("."));
    let temp_path = parent.join(format!(
        ".progress_tmp_{}",
        std::process::id() // Use PID for uniqueness
    ));

    // Write to temp file
    {
        let mut file = std::fs::File::create(&temp_path).map_err(SummarizeError::CreateTempFile)?;
        file.write_all(content.as_bytes())
            .map_err(SummarizeError::WriteFile)?;
        file.sync_all().map_err(SummarizeError::WriteFile)?;
    }

    // Atomic rename
    std::fs::rename(&temp_path, path).map_err(SummarizeError::RenameTempFile)
}

/// Execute summarization command and return the output.
///
/// This is the imperative shell function that invokes the external command.
pub fn execute_summarize_command(command: &str) -> Result<String, SummarizeError> {
    // Use sh -c to execute the command string
    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(SummarizeError::SpawnCommand)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);
        return Err(SummarizeError::CommandFailed { exit_code, stderr });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // Validate non-empty output
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(SummarizeError::EmptyContent);
    }

    // Return the output with trailing newline if not present
    if stdout.ends_with('\n') {
        Ok(stdout)
    } else {
        Ok(format!("{}\n", stdout))
    }
}

/// Attempt to summarize the progress file if needed.
///
/// This is the main entry point for the summarization feature.
/// It checks if summarization is needed, invokes the command, and replaces
/// the progress file with the summarized content.
///
/// Returns Ok(true) if summarization was performed, Ok(false) if not needed,
/// or an error if summarization failed.
///
/// Shows a spinner with elapsed time during the summarization subprocess
/// when stdout is a terminal.
pub fn maybe_summarize_progress(
    progress_path: &Path,
    config: &SummarizeConfig,
) -> Result<bool, SummarizeError> {
    // Read current content
    let content = read_progress_file(progress_path)?;

    // Check if summarization is needed
    let check = check_summarization_needed(&content, config);
    if !check.needed {
        return Ok(false);
    }

    // Build the command
    let progress_path_str = progress_path.display().to_string();
    let command = build_summarize_command(config, &progress_path_str, &content);

    // Create spinner for the summarization process
    let mut spinner = Spinner::new();

    // Start spinner with Summarizing context
    spinner.start_with_context(SpinnerContext::Summarizing);

    // Execute summarization
    let result = execute_summarize_command(&command);

    // Stop spinner before showing result message
    spinner.stop();

    // Handle result
    let summarized = result?;

    // Write atomically
    write_progress_file_atomic(progress_path, &summarized)?;

    let new_line_count = count_lines(&summarized);
    eprintln!(
        "Progress file summarized: {} lines → {} lines",
        check.line_count, new_line_count
    );

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_lines_empty() {
        assert_eq!(count_lines(""), 0);
    }

    #[test]
    fn test_count_lines_single() {
        assert_eq!(count_lines("one line"), 1);
    }

    #[test]
    fn test_count_lines_multiple() {
        assert_eq!(count_lines("line1\nline2\nline3"), 3);
    }

    #[test]
    fn test_count_lines_with_trailing_newline() {
        assert_eq!(count_lines("line1\nline2\n"), 2);
    }

    #[test]
    fn test_count_lines_blank_lines() {
        assert_eq!(count_lines("line1\n\nline3"), 3);
    }

    #[test]
    fn test_summarize_config_default() {
        let config = SummarizeConfig::default();
        assert_eq!(config.max_lines, 1000);
        assert!(!config.disabled);
        assert!(config.should_summarize());
    }

    #[test]
    fn test_summarize_config_disabled() {
        let config = SummarizeConfig {
            disabled: true,
            ..Default::default()
        };
        assert!(!config.should_summarize());
    }

    #[test]
    fn test_summarize_config_zero_max_lines() {
        let config = SummarizeConfig {
            max_lines: 0,
            ..Default::default()
        };
        assert!(!config.should_summarize());
    }

    #[test]
    fn test_check_summarization_not_needed() {
        let config = SummarizeConfig::default();
        let content = "line1\nline2\nline3";
        let check = check_summarization_needed(content, &config);
        assert!(!check.needed);
        assert_eq!(check.line_count, 3);
    }

    #[test]
    fn test_check_summarization_needed() {
        let config = SummarizeConfig {
            max_lines: 5,
            ..Default::default()
        };
        let content = "1\n2\n3\n4\n5\n6\n7"; // 7 lines
        let check = check_summarization_needed(content, &config);
        assert!(check.needed);
        assert_eq!(check.line_count, 7);
        assert_eq!(check.threshold, 5);
    }

    #[test]
    fn test_check_summarization_at_threshold() {
        let config = SummarizeConfig {
            max_lines: 5,
            ..Default::default()
        };
        let content = "1\n2\n3\n4\n5"; // exactly 5 lines
        let check = check_summarization_needed(content, &config);
        assert!(!check.needed); // threshold is >5, not >=5
    }

    #[test]
    fn test_check_summarization_disabled() {
        let config = SummarizeConfig {
            max_lines: 5,
            disabled: true,
            ..Default::default()
        };
        let content = "1\n2\n3\n4\n5\n6\n7"; // 7 lines
        let check = check_summarization_needed(content, &config);
        assert!(!check.needed);
    }

    #[test]
    fn test_build_summarize_command_basic() {
        let config = SummarizeConfig {
            command: "echo {prompt}".to_string(),
            prompt: "Summarize: {progress_content}".to_string(),
            ..Default::default()
        };
        let result = build_summarize_command(&config, "/path/file.txt", "content here");
        assert_eq!(result, "echo 'Summarize: content here'");
    }

    #[test]
    fn test_build_summarize_command_with_quotes() {
        let config = SummarizeConfig {
            command: "echo {prompt}".to_string(),
            prompt: "It's a test".to_string(),
            ..Default::default()
        };
        let result = build_summarize_command(&config, "/path", "");
        assert_eq!(result, "echo 'It'\"'\"'s a test'");
    }

    #[test]
    fn test_build_summarize_command_with_path_placeholder() {
        let config = SummarizeConfig {
            command: "claude -p {prompt}".to_string(),
            prompt: "File: {progress_file}".to_string(),
            ..Default::default()
        };
        let result = build_summarize_command(&config, "/my/path.txt", "");
        assert_eq!(result, "claude -p 'File: /my/path.txt'");
    }

    #[test]
    fn test_build_summarize_command_with_both_placeholders() {
        let config = SummarizeConfig {
            command: "cmd {prompt}".to_string(),
            prompt: "Path: {progress_file}\nContent: {progress_content}".to_string(),
            ..Default::default()
        };
        let result = build_summarize_command(&config, "/p.txt", "hello");
        assert_eq!(result, "cmd 'Path: /p.txt\nContent: hello'");
    }

    #[test]
    fn test_build_summarize_command_default_templates() {
        let config = SummarizeConfig::default();
        let result = build_summarize_command(&config, "/progress.txt", "line 1\nline 2");

        // Should contain the path and content
        assert!(result.contains("/progress.txt"));
        assert!(result.contains("line 1"));
        assert!(result.contains("line 2"));
        // Should be a claude command with permission bypass
        assert!(result.starts_with("claude --dangerously-skip-permissions -p '"));
    }
}
