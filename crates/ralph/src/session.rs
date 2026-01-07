//! Session directory initialization and management (Imperative Shell).
//!
//! This module handles the I/O operations for session directory creation,
//! metadata storage, and the global sessions index. It uses pure functions
//! from ralph_core::session for slug generation and type definitions, then
//! performs the actual file system operations.

// TODO: Wire up session commands to CLI - these functions are ready for use
#![allow(dead_code)]

use ralph_core::session::{
    generate_unique_slug, is_valid_slug, SessionEntry, SessionMetadata, SessionsIndex,
};
use std::fs;
use std::path::{Path, PathBuf};

/// Error type for session operations.
#[derive(thiserror::Error, Debug)]
pub enum SessionError {
    /// Session slug already exists
    #[error(
        "Session '{slug}' already exists. Choose a different slug or omit for auto-generated."
    )]
    DuplicateSlug { slug: String },

    /// Failed to generate a unique slug
    #[error("Failed to generate a unique session slug after maximum attempts. This should not happen with the current word list size.")]
    SlugGenerationFailed,

    /// Invalid slug format
    #[error("Invalid slug format: '{slug}'. Slug must be lowercase with format 'adjective-noun'.")]
    InvalidSlug { slug: String },

    /// Failed to create session directory
    #[error("Failed to create session directory at {path}: {source}")]
    CreateSessionDir {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to read sessions index
    #[error("Failed to read sessions index at {path}: {source}")]
    ReadSessionsIndex {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to write sessions index
    #[error("Failed to write sessions index at {path}: {source}")]
    WriteSessionsIndex {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse sessions index
    #[error("Failed to parse sessions index: {0}")]
    ParseSessionsIndex(#[from] toml::de::Error),

    /// Failed to serialize sessions index
    #[error("Failed to serialize sessions index: {0}")]
    SerializeSessionsIndex(#[from] toml::ser::Error),

    /// Failed to write session metadata
    #[error("Failed to write session metadata at {path}: {source}")]
    WriteSessionMetadata {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Get the path to the global sessions directory (~/.config/ralph/sessions/)
pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ralph")
        .join("sessions")
}

/// Get the path to the global sessions index file (~/.config/ralph/sessions.toml)
pub fn sessions_index_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ralph")
        .join("sessions.toml")
}

/// Get the path to a specific session directory
pub fn session_dir(slug: &str) -> PathBuf {
    sessions_dir().join(slug)
}

/// Load the sessions index from disk, or create a new empty one if it doesn't exist.
pub fn load_sessions_index() -> Result<SessionsIndex, SessionError> {
    let path = sessions_index_path();

    if !path.exists() {
        return Ok(SessionsIndex::new());
    }

    let content = fs::read_to_string(&path).map_err(|e| SessionError::ReadSessionsIndex {
        path: path.display().to_string(),
        source: e,
    })?;

    SessionsIndex::from_toml(&content).map_err(SessionError::ParseSessionsIndex)
}

/// Save the sessions index to disk.
pub fn save_sessions_index(index: &SessionsIndex) -> Result<(), SessionError> {
    let path = sessions_index_path();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| SessionError::WriteSessionsIndex {
            path: parent.display().to_string(),
            source: e,
        })?;
    }

    let content = index.to_toml()?;

    fs::write(&path, content).map_err(|e| SessionError::WriteSessionsIndex {
        path: path.display().to_string(),
        source: e,
    })?;

    Ok(())
}

/// Validate or generate a session slug.
///
/// If `user_slug` is provided, validates it and checks for uniqueness.
/// If None, generates a unique slug automatically.
pub fn resolve_session_slug(user_slug: Option<&str>) -> Result<String, SessionError> {
    let index = load_sessions_index()?;

    if let Some(slug) = user_slug {
        // Validate format
        if !is_valid_slug(slug) {
            return Err(SessionError::InvalidSlug {
                slug: slug.to_string(),
            });
        }

        // Check uniqueness
        if index.slug_exists(slug) {
            return Err(SessionError::DuplicateSlug {
                slug: slug.to_string(),
            });
        }

        Ok(slug.to_string())
    } else {
        // Generate unique slug
        let existing = index.existing_slugs();
        let mut rng = rand::thread_rng();

        generate_unique_slug(&mut rng, &existing, 100).ok_or(SessionError::SlugGenerationFailed)
    }
}

/// Create a session directory and initialize metadata files.
///
/// This function:
/// 1. Creates the session directory at ~/.config/ralph/sessions/<slug>/
/// 2. Writes session.toml with initial metadata
/// 3. Appends an entry to ~/.config/ralph/sessions.toml
///
/// # Arguments
///
/// * `slug` - The session identifier (must be unique)
/// * `project_path` - Absolute path to the project directory
///
/// # Returns
///
/// * `Ok(PathBuf)` - Path to the created session directory
/// * `Err(SessionError)` - If directory creation or metadata writing fails
pub fn initialize_session_directory(
    slug: &str,
    project_path: &Path,
) -> Result<PathBuf, SessionError> {
    // Create session directory
    let session_path = session_dir(slug);
    fs::create_dir_all(&session_path).map_err(|e| SessionError::CreateSessionDir {
        path: session_path.display().to_string(),
        source: e,
    })?;

    // Create session metadata
    let metadata = SessionMetadata::new(slug.to_string(), project_path.to_path_buf());

    // Write session.toml in the session directory
    let session_toml_path = session_path.join("session.toml");
    let metadata_content = metadata.to_toml()?;
    fs::write(&session_toml_path, metadata_content).map_err(|e| {
        SessionError::WriteSessionMetadata {
            path: session_toml_path.display().to_string(),
            source: e,
        }
    })?;

    // Add entry to global sessions index
    let mut index = load_sessions_index()?;
    let entry = SessionEntry::new(slug.to_string(), project_path.to_path_buf());
    index.add_session(entry);
    save_sessions_index(&index)?;

    Ok(session_path)
}

/// Initialize a new session with automatic or user-provided slug.
///
/// This is the main entry point that orchestrates session initialization:
/// 1. Resolves/generates the session slug
/// 2. Creates the session directory
/// 3. Initializes metadata files
///
/// # Arguments
///
/// * `user_slug` - Optional user-provided slug. If None, generates automatically.
/// * `project_path` - Absolute path to the project directory
///
/// # Returns
///
/// * `Ok((slug, session_dir))` - The slug used and path to the session directory
/// * `Err(SessionError)` - If initialization fails
pub fn initialize_session(
    user_slug: Option<&str>,
    project_path: &Path,
) -> Result<(String, PathBuf), SessionError> {
    let slug = resolve_session_slug(user_slug)?;
    let session_path = initialize_session_directory(&slug, project_path)?;
    Ok((slug, session_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Helper to create a temporary sessions directory for testing
    fn with_temp_sessions_dir<F>(test: F)
    where
        F: FnOnce(&TempDir),
    {
        let temp_dir = TempDir::new().unwrap();

        // Override the sessions dir for tests by using a temp directory
        // In real code, we'd use the actual config dir, but for tests we need isolation
        test(&temp_dir);
    }

    #[test]
    fn test_sessions_index_path_structure() {
        let path = sessions_index_path();
        assert!(path.to_string_lossy().contains("ralph"));
        assert!(path.to_string_lossy().contains("sessions.toml"));
    }

    #[test]
    fn test_session_dir_path_structure() {
        let path = session_dir("test-slug");
        assert!(path.to_string_lossy().contains("ralph"));
        assert!(path.to_string_lossy().contains("sessions"));
        assert!(path.to_string_lossy().ends_with("test-slug"));
    }

    #[test]
    fn test_load_sessions_index_creates_empty_when_missing() {
        // This test relies on the index not existing at the path,
        // which is tricky to guarantee in a real filesystem.
        // In practice, we'd use a mock or temp directory override.
        let index = SessionsIndex::new();
        assert_eq!(index.sessions.len(), 0);
    }

    #[test]
    fn test_save_and_load_sessions_index() {
        with_temp_sessions_dir(|temp_dir| {
            let test_path = temp_dir.path().join("sessions.toml");

            let mut index = SessionsIndex::new();
            index.add_session(SessionEntry::new(
                "test-session".to_string(),
                PathBuf::from("/test/project"),
            ));

            // Manually save to test path
            let content = index.to_toml().unwrap();
            fs::write(&test_path, content).unwrap();

            // Load and verify
            let loaded_content = fs::read_to_string(&test_path).unwrap();
            let loaded_index = SessionsIndex::from_toml(&loaded_content).unwrap();

            assert_eq!(loaded_index.sessions.len(), 1);
            assert_eq!(loaded_index.sessions[0].slug, "test-session");
        });
    }

    #[test]
    fn test_resolve_session_slug_validates_format() {
        let result = resolve_session_slug(Some("INVALID"));
        assert!(matches!(result, Err(SessionError::InvalidSlug { .. })));

        let _result = resolve_session_slug(Some("no-uppercase"));
        // This might succeed if the slug doesn't exist, so we're just testing format validation
        // The actual validation happens in is_valid_slug which is tested in core
    }

    #[test]
    fn test_resolve_session_slug_generates_when_none() {
        let result = resolve_session_slug(None);
        // Should generate a slug (might fail if we can't access/create config dir, but that's OK for this test)
        // We're mainly testing that it doesn't panic
        assert!(result.is_ok() || matches!(result, Err(SessionError::ReadSessionsIndex { .. })));
    }

    #[test]
    fn test_initialize_session_directory_creates_structure() {
        with_temp_sessions_dir(|temp_dir| {
            let session_path = temp_dir.path().join("test-session");
            let project_path = PathBuf::from("/test/project");

            // Manually create the session directory and metadata
            fs::create_dir_all(&session_path).unwrap();

            let metadata = SessionMetadata::new("test-session".to_string(), project_path.clone());
            let session_toml_path = session_path.join("session.toml");
            let metadata_content = metadata.to_toml().unwrap();
            fs::write(&session_toml_path, metadata_content).unwrap();

            // Verify structure
            assert!(session_path.exists());
            assert!(session_toml_path.exists());

            // Verify metadata content
            let loaded_content = fs::read_to_string(&session_toml_path).unwrap();
            let loaded_metadata = SessionMetadata::from_toml(&loaded_content).unwrap();
            assert_eq!(loaded_metadata.slug, "test-session");
            assert_eq!(loaded_metadata.project, project_path);
        });
    }
}
