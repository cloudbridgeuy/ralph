//! Git diff capture functionality.
//!
//! This module handles capturing git diffs after each iteration for auditing purposes.
//! It follows the Imperative Shell pattern - all functions perform I/O operations.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// Errors that can occur during git operations.
#[derive(Debug, Error)]
pub enum GitError {
    /// Failed to execute git command
    #[error("Failed to execute git command: {0}")]
    CommandFailed(#[from] io::Error),

    /// Git command returned non-zero exit code
    #[error("Git command failed with exit code {code}: {stderr}")]
    GitFailed { code: i32, stderr: String },

    /// Not a git repository
    #[error("Not a git repository: {0}")]
    NotGitRepository(String),

    /// Failed to write diff file
    #[error("Failed to write diff file: {0}")]
    WriteFailed(io::Error),
}

/// Result of capturing a git diff.
#[derive(Debug)]
pub struct GitDiff {
    /// The diff content (may be empty if no changes)
    pub content: String,
    /// Whether the current directory is a git repository
    pub is_git_repo: bool,
}

/// Checks if the current directory (or any parent) is a git repository.
///
/// # Errors
///
/// Returns an error if the git command fails to execute.
///
/// # Examples
///
/// ```no_run
/// use ralph::git::is_git_repository;
///
/// match is_git_repository() {
///     Ok(true) => println!("In a git repository"),
///     Ok(false) => println!("Not in a git repository"),
///     Err(e) => eprintln!("Error checking git: {}", e),
/// }
/// ```
pub fn is_git_repository() -> Result<bool, GitError> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()?;

    Ok(output.status.success())
}

/// Captures the current git diff (both staged and unstaged changes).
///
/// # Errors
///
/// Returns an error if:
/// - The git command fails to execute
/// - The git command returns a non-zero exit code (except when not a repo)
///
/// # Examples
///
/// ```no_run
/// use ralph::git::capture_git_diff;
///
/// match capture_git_diff() {
///     Ok(diff) if diff.is_git_repo => {
///         if diff.content.is_empty() {
///             println!("No changes");
///         } else {
///             println!("Diff: {}", diff.content);
///         }
///     }
///     Ok(_) => println!("Not a git repository"),
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// ```
pub fn capture_git_diff() -> Result<GitDiff, GitError> {
    // First check if we're in a git repository
    if !is_git_repository()? {
        return Ok(GitDiff {
            content: String::new(),
            is_git_repo: false,
        });
    }

    // Capture both staged and unstaged changes
    let output = Command::new("git")
        .args(["diff", "HEAD"])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::GitFailed {
            code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    let content = String::from_utf8_lossy(&output.stdout).to_string();

    Ok(GitDiff {
        content,
        is_git_repo: true,
    })
}

/// Writes a git diff to a file.
///
/// Creates parent directories if they don't exist.
///
/// # Arguments
///
/// * `diff_path` - Path where the diff file should be written
/// * `diff_content` - The diff content to write
///
/// # Errors
///
/// Returns an error if:
/// - Parent directory creation fails
/// - File write operation fails
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
/// use ralph::git::write_diff_file;
///
/// let path = PathBuf::from("/tmp/iteration-1.diff");
/// let content = "diff --git a/file.rs b/file.rs...";
/// write_diff_file(&path, content).expect("Failed to write diff");
/// ```
pub fn write_diff_file(diff_path: &Path, diff_content: &str) -> Result<(), GitError> {
    // Create parent directory if needed
    if let Some(parent) = diff_path.parent() {
        std::fs::create_dir_all(parent).map_err(GitError::WriteFailed)?;
    }

    // Write the diff file
    let mut file = std::fs::File::create(diff_path).map_err(GitError::WriteFailed)?;
    file.write_all(diff_content.as_bytes())
        .map_err(GitError::WriteFailed)?;

    Ok(())
}

/// Captures git diff and writes it to a file.
///
/// This is a convenience function that combines capturing and writing.
/// If not in a git repository, writes an empty file and logs a warning to stderr.
///
/// # Arguments
///
/// * `diff_path` - Path where the diff file should be written
///
/// # Errors
///
/// Returns an error if:
/// - Git command execution fails
/// - File write operation fails
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
/// use ralph::git::capture_and_write_diff;
///
/// let path = PathBuf::from("~/.config/ralph/sessions/my-session/iteration-1.diff");
/// capture_and_write_diff(&path).expect("Failed to capture diff");
/// ```
pub fn capture_and_write_diff(diff_path: &Path) -> Result<(), GitError> {
    let diff = capture_git_diff()?;

    if !diff.is_git_repo {
        // Not a git repository - write empty file and warn
        write_diff_file(diff_path, "")?;
        eprintln!("Warning: Not a git repository. Skipping diff capture.");
        return Ok(());
    }

    write_diff_file(diff_path, &diff.content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_is_git_repository_in_non_git_dir() {
        // Create a temporary directory that's not a git repo
        let temp_dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();

        std::env::set_current_dir(temp_dir.path()).unwrap();
        let result = is_git_repository();
        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_capture_git_diff_in_non_git_dir() {
        // Create a temporary directory that's not a git repo
        let temp_dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();

        std::env::set_current_dir(temp_dir.path()).unwrap();
        let result = capture_git_diff();
        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_ok());
        let diff = result.unwrap();
        assert!(!diff.is_git_repo);
        assert!(diff.content.is_empty());
    }

    #[test]
    fn test_write_diff_file_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let diff_path = temp_dir.path().join("sessions/my-session/iteration-1.diff");

        let content = "diff --git a/test.rs b/test.rs\n--- a/test.rs\n+++ b/test.rs";
        let result = write_diff_file(&diff_path, content);

        assert!(result.is_ok());
        assert!(diff_path.exists());

        let written = fs::read_to_string(&diff_path).unwrap();
        assert_eq!(written, content);
    }

    #[test]
    fn test_write_diff_file_empty_content() {
        let temp_dir = TempDir::new().unwrap();
        let diff_path = temp_dir.path().join("iteration-1.diff");

        let result = write_diff_file(&diff_path, "");

        assert!(result.is_ok());
        assert!(diff_path.exists());

        let written = fs::read_to_string(&diff_path).unwrap();
        assert_eq!(written, "");
    }

    #[test]
    fn test_write_diff_file_overwrites_existing() {
        let temp_dir = TempDir::new().unwrap();
        let diff_path = temp_dir.path().join("iteration-1.diff");

        // Write first content
        write_diff_file(&diff_path, "first content").unwrap();
        assert_eq!(fs::read_to_string(&diff_path).unwrap(), "first content");

        // Overwrite with second content
        write_diff_file(&diff_path, "second content").unwrap();
        assert_eq!(fs::read_to_string(&diff_path).unwrap(), "second content");
    }

    #[test]
    fn test_capture_and_write_diff_in_non_git_dir() {
        let temp_dir = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();

        let diff_path = temp_dir.path().join("iteration-1.diff");

        std::env::set_current_dir(temp_dir.path()).unwrap();
        let result = capture_and_write_diff(&diff_path);
        std::env::set_current_dir(original_dir).unwrap();

        assert!(result.is_ok());
        assert!(diff_path.exists());

        // Should write empty file when not in git repo
        let written = fs::read_to_string(&diff_path).unwrap();
        assert_eq!(written, "");
    }

    #[test]
    fn test_git_diff_structure() {
        let diff = GitDiff {
            content: "some diff".to_string(),
            is_git_repo: true,
        };

        assert_eq!(diff.content, "some diff");
        assert!(diff.is_git_repo);
    }
}
