//! Context file initialization (Imperative Shell).
//!
//! This module handles the I/O operations for initializing context files.
//! It uses the pure functions from ralph_core::context to determine what
//! needs to be done, then performs the actual file system operations.

use ralph_core::context::{ContextFilesTouch, ContextPaths};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Error type for context file initialization.
#[derive(thiserror::Error, Debug)]
pub enum InitError {
    /// PRD file does not exist
    #[error("PRD file not found at {path}. Create it with user stories to begin.")]
    PrdNotFound { path: String },

    /// Failed to create directory
    #[error("Failed to create directory {path}: {source}")]
    CreateDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to create file
    #[error("Failed to create file {path}: {source}")]
    CreateFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Check if the PRD file exists, returning an error if not.
///
/// Unlike design doc and progress file, the PRD must exist before
/// initialization can proceed.
pub fn verify_prd_exists(paths: &ContextPaths) -> Result<(), InitError> {
    if !paths.prd.exists() {
        return Err(InitError::PrdNotFound {
            path: paths.prd.display().to_string(),
        });
    }
    Ok(())
}

/// Check which context files exist and return existence flags.
///
/// Returns (design_exists, progress_exists) for use with the core
/// function determine_files_to_touch.
pub fn check_context_files_exist(paths: &ContextPaths) -> (bool, bool) {
    (paths.design.exists(), paths.progress.exists())
}

/// Touch (create) missing context files.
///
/// Creates empty files for any context files that don't exist,
/// including their parent directories. Prints a notice to stderr
/// for each file created.
///
/// # Arguments
///
/// * `to_touch` - The result from determine_files_to_touch indicating which files to create
///
/// # Returns
///
/// * `Ok(())` - All files were created successfully
/// * `Err(InitError)` - Failed to create a directory or file
pub fn touch_context_files(to_touch: &ContextFilesTouch) -> Result<(), InitError> {
    for path in to_touch.files_to_create() {
        touch_file(path)?;
        eprintln!("Created missing context file: {}", path.display());
    }
    Ok(())
}

/// Create an empty file at the given path, creating parent directories as needed.
fn touch_file(path: &Path) -> Result<(), InitError> {
    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| InitError::CreateDir {
                path: parent.display().to_string(),
                source: e,
            })?;
        }
    }

    // Create empty file
    let mut file = fs::File::create(path).map_err(|e| InitError::CreateFile {
        path: path.display().to_string(),
        source: e,
    })?;

    // Ensure the file is flushed (touch semantics)
    file.flush().map_err(|e| InitError::CreateFile {
        path: path.display().to_string(),
        source: e,
    })?;

    Ok(())
}

/// Initialize context files for a run session.
///
/// This is the main entry point that orchestrates the initialization:
/// 1. Verifies the PRD exists (error if not)
/// 2. Checks which other context files exist
/// 3. Creates any missing design doc or progress file
///
/// # Arguments
///
/// * `paths` - The resolved context file paths
///
/// # Returns
///
/// * `Ok(())` - Initialization succeeded
/// * `Err(InitError)` - PRD missing or failed to create files
pub fn initialize_context_files(paths: &ContextPaths) -> Result<(), InitError> {
    // PRD must exist - error if missing
    verify_prd_exists(paths)?;

    // Check which files exist
    let (design_exists, progress_exists) = check_context_files_exist(paths);

    // Determine which files need to be touched
    let to_touch =
        ralph_core::context::determine_files_to_touch(paths, design_exists, progress_exists);

    // Touch missing files
    if to_touch.has_files_to_create() {
        touch_context_files(&to_touch)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_paths(temp_dir: &TempDir) -> ContextPaths {
        ContextPaths {
            design: temp_dir.path().join(".claude/designs/design.md"),
            prd: temp_dir.path().join(".claude/plans/prd.toml"),
            progress: temp_dir.path().join(".claude/plans/progress.txt"),
        }
    }

    #[test]
    fn test_verify_prd_exists_when_missing() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        let result = verify_prd_exists(&paths);
        assert!(matches!(result, Err(InitError::PrdNotFound { .. })));
    }

    #[test]
    fn test_verify_prd_exists_when_present() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        // Create PRD file
        fs::create_dir_all(paths.prd.parent().unwrap()).unwrap();
        fs::write(&paths.prd, "[[stories]]\npasses = false").unwrap();

        let result = verify_prd_exists(&paths);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_context_files_exist() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        // Initially nothing exists
        let (design, progress) = check_context_files_exist(&paths);
        assert!(!design);
        assert!(!progress);

        // Create design file
        fs::create_dir_all(paths.design.parent().unwrap()).unwrap();
        fs::write(&paths.design, "").unwrap();

        let (design, progress) = check_context_files_exist(&paths);
        assert!(design);
        assert!(!progress);
    }

    #[test]
    fn test_touch_context_files_creates_missing() {
        let temp_dir = TempDir::new().unwrap();
        let to_touch = ContextFilesTouch {
            design: Some(temp_dir.path().join(".claude/designs/design.md")),
            progress: Some(temp_dir.path().join(".claude/plans/progress.txt")),
        };

        let result = touch_context_files(&to_touch);
        assert!(result.is_ok());

        assert!(temp_dir.path().join(".claude/designs/design.md").exists());
        assert!(temp_dir.path().join(".claude/plans/progress.txt").exists());
    }

    #[test]
    fn test_touch_context_files_creates_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let deep_path = temp_dir.path().join("a/b/c/d/file.txt");
        let to_touch = ContextFilesTouch {
            design: Some(deep_path.clone()),
            progress: None,
        };

        let result = touch_context_files(&to_touch);
        assert!(result.is_ok());
        assert!(deep_path.exists());
    }

    #[test]
    fn test_initialize_context_files_fails_without_prd() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        let result = initialize_context_files(&paths);
        assert!(matches!(result, Err(InitError::PrdNotFound { .. })));
    }

    #[test]
    fn test_initialize_context_files_creates_missing_files() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        // Create PRD file
        fs::create_dir_all(paths.prd.parent().unwrap()).unwrap();
        fs::write(&paths.prd, "[[stories]]\npasses = false").unwrap();

        let result = initialize_context_files(&paths);
        assert!(result.is_ok());

        // Design and progress should now exist
        assert!(paths.design.exists());
        assert!(paths.progress.exists());
    }

    #[test]
    fn test_initialize_context_files_preserves_existing() {
        let temp_dir = TempDir::new().unwrap();
        let paths = create_test_paths(&temp_dir);

        // Create all files with content
        fs::create_dir_all(paths.prd.parent().unwrap()).unwrap();
        fs::create_dir_all(paths.design.parent().unwrap()).unwrap();
        fs::write(&paths.prd, "[[stories]]\npasses = false").unwrap();
        fs::write(&paths.design, "existing design content").unwrap();
        fs::write(&paths.progress, "existing progress content").unwrap();

        let result = initialize_context_files(&paths);
        assert!(result.is_ok());

        // Content should be preserved
        assert_eq!(
            fs::read_to_string(&paths.design).unwrap(),
            "existing design content"
        );
        assert_eq!(
            fs::read_to_string(&paths.progress).unwrap(),
            "existing progress content"
        );
    }
}
