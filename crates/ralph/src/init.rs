//! PRD file verification (Imperative Shell).
//!
//! This module handles verifying the PRD file exists before running.
//! In the simplified context model, only the PRD is required for initialization.
//! Progress notes are optional and managed separately.

use std::path::Path;

/// Error type for initialization.
#[derive(thiserror::Error, Debug)]
pub enum InitError {
    /// PRD file does not exist
    #[error("PRD file not found at {path}. Create it with user stories to begin.")]
    PrdNotFound { path: String },
}

/// Verify the PRD file exists.
///
/// # Arguments
///
/// * `prd_path` - Path to the PRD file
///
/// # Returns
///
/// * `Ok(())` - PRD exists
/// * `Err(InitError::PrdNotFound)` - PRD does not exist
pub fn verify_prd_exists(prd_path: &Path) -> Result<(), InitError> {
    if !prd_path.exists() {
        return Err(InitError::PrdNotFound {
            path: prd_path.display().to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_verify_prd_exists_when_missing() {
        let temp_dir = TempDir::new().unwrap();
        let prd_path = temp_dir.path().join(".local/plans/prd.toml");

        let result = verify_prd_exists(&prd_path);
        assert!(matches!(result, Err(InitError::PrdNotFound { .. })));
    }

    #[test]
    fn test_verify_prd_exists_when_present() {
        let temp_dir = TempDir::new().unwrap();
        let prd_path = temp_dir.path().join(".local/plans/prd.toml");

        // Create PRD file
        fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
        fs::write(&prd_path, "[[stories]]\npasses = false").unwrap();

        let result = verify_prd_exists(&prd_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_prd_exists_error_contains_path() {
        let temp_dir = TempDir::new().unwrap();
        let prd_path = temp_dir.path().join("custom/prd.toml");

        let result = verify_prd_exists(&prd_path);
        match result {
            Err(InitError::PrdNotFound { path }) => {
                assert!(path.contains("custom/prd.toml"));
            }
            Ok(()) => panic!("Expected error"),
        }
    }
}
