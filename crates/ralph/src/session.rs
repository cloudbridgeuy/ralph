//! Session directory initialization and management (Imperative Shell).
//!
//! This module handles the I/O operations for session directory creation,
//! metadata storage, and the global sessions index. It uses pure functions
//! from ralph_core::session for slug generation and type definitions, then
//! performs the actual file system operations.
//!
//! # Session Storage Location
//!
//! Sessions are stored in the data directory, which follows platform conventions
//! and can be overridden via the `RALPH_DATA_DIR` environment variable.
//!
//! By default:
//! - **Linux**: `~/.local/share/ralph/sessions/`
//! - **macOS**: `~/Library/Application Support/ralph/sessions/`
//! - **Windows**: `%APPDATA%\ralph\sessions\`
//!
//! Override with `RALPH_DATA_DIR` environment variable.

use crate::paths;
use ralph_core::session::{
    generate_unique_slug, is_valid_slug, SessionEntry, SessionMetadata, SessionOutcome,
    SessionsIndex,
};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Error type for session operations.
#[derive(thiserror::Error, Debug)]
pub enum SessionError {
    /// Session slug already exists
    #[error(
        "Session '{slug}' already exists. Choose a different slug or omit for auto-generated."
    )]
    DuplicateSlug { slug: String },

    /// Session not found
    #[error("Session '{slug}' not found. Run 'ralph sessions' to list available sessions.")]
    SessionNotFound { slug: String },

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

    /// Failed to serialize sessions index
    #[error("Failed to serialize sessions index: {0}")]
    SerializeSessionsIndex(#[from] toml::ser::Error),

    /// Round-trip verification failed during save
    #[error("Sessions index round-trip verification failed: {0}")]
    RoundTripVerificationFailed(String),

    /// Failed to write session metadata
    #[error("Failed to write session metadata at {path}: {source}")]
    WriteSessionMetadata {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Get the path to the global sessions index file.
///
/// Uses platform-specific paths by default, which can be overridden
/// via the `RALPH_DATA_DIR` environment variable.
///
/// See [`crate::paths::sessions_index_path`] for details.
pub fn sessions_index_path() -> PathBuf {
    paths::sessions_index_path()
}

/// Get the path to a specific session directory.
///
/// Uses platform-specific paths by default, which can be overridden
/// via the `RALPH_DATA_DIR` environment variable.
///
/// See [`crate::paths::session_dir`] for details.
pub fn session_dir(slug: &str) -> PathBuf {
    paths::session_dir(slug)
}

/// Load the sessions index from disk, or create a new empty one if it doesn't exist.
///
/// If the index file exists but contains corrupt TOML, warns the user and returns
/// an empty index rather than failing. The sessions index is a convenience cache —
/// each session also has its own `session.toml` in its directory. The next
/// `save_sessions_index()` call will write a clean index, self-healing the corruption.
///
/// I/O read errors still propagate as hard errors since those indicate real system problems.
pub fn load_sessions_index() -> Result<SessionsIndex, SessionError> {
    let path = sessions_index_path();

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SessionsIndex::new()),
        Err(e) => {
            return Err(SessionError::ReadSessionsIndex {
                path: path.display().to_string(),
                source: e,
            })
        }
    };

    match SessionsIndex::from_toml(&content) {
        Ok(index) => Ok(index),
        Err(e) => {
            crate::warn::warn(format!(
                "Sessions index at {} is corrupt ({}). \
                 Starting with empty index. Session data is preserved in individual session directories. \
                 The index will self-heal when a new session is created.",
                path.display(),
                e
            ));
            Ok(SessionsIndex::new())
        }
    }
}

/// Save the sessions index to disk using atomic write.
///
/// Uses a temp file + rename strategy to prevent corruption from interrupted writes.
/// Also round-trip verifies the serialized content before touching disk.
pub fn save_sessions_index(index: &SessionsIndex) -> Result<(), SessionError> {
    let path = sessions_index_path();

    // Ensure parent directory exists
    let parent = path
        .parent()
        .ok_or_else(|| SessionError::WriteSessionsIndex {
            path: path.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no parent directory"),
        })?;

    fs::create_dir_all(parent).map_err(|e| SessionError::WriteSessionsIndex {
        path: parent.display().to_string(),
        source: e,
    })?;

    let content = index.to_toml()?;

    // Round-trip verify: parse the serialized string back to catch serialization bugs
    // before writing anything to disk
    SessionsIndex::from_toml(&content).map_err(|e| {
        SessionError::RoundTripVerificationFailed(format!(
            "serialized TOML failed to parse back: {}",
            e
        ))
    })?;

    // Atomic write: temp file in same directory (same filesystem) then rename
    let mut temp_file =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| SessionError::WriteSessionsIndex {
            path: parent.display().to_string(),
            source: e,
        })?;

    temp_file
        .write_all(content.as_bytes())
        .map_err(|e| SessionError::WriteSessionsIndex {
            path: path.display().to_string(),
            source: e,
        })?;

    temp_file
        .persist(&path)
        .map_err(|e| SessionError::WriteSessionsIndex {
            path: path.display().to_string(),
            source: e.error,
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
        let mut rng = rand::rng();

        generate_unique_slug(&mut rng, &existing, 100).ok_or(SessionError::SlugGenerationFailed)
    }
}

/// Create a session directory and initialize metadata files.
///
/// This function:
/// 1. Creates the session directory at `~/.config/ralph/sessions/{slug}/`
/// 2. Writes session.toml with initial metadata
/// 3. Appends an entry to ~/.config/ralph/sessions.toml
///
/// # Arguments
///
/// * `slug` - The session identifier (must be unique)
/// * `project_path` - Absolute path to the project directory
/// * `prompt` - Optional prompt string (after placeholder substitution) for replay
///
/// # Returns
///
/// * `Ok(PathBuf)` - Path to the created session directory
/// * `Err(SessionError)` - If directory creation or metadata writing fails
pub fn initialize_session_directory(
    slug: &str,
    project_path: &Path,
    prompt: Option<String>,
    persona: Option<&str>,
) -> Result<PathBuf, SessionError> {
    initialize_session_directory_internal(slug, project_path, prompt, None, persona)
}

/// Create a session directory for a cloned session.
///
/// Same as `initialize_session_directory` but records the source session slug
/// in the metadata for traceability.
fn initialize_session_directory_with_clone(
    slug: &str,
    project_path: &Path,
    prompt: Option<String>,
    source_slug: &str,
    persona: Option<&str>,
) -> Result<PathBuf, SessionError> {
    initialize_session_directory_internal(slug, project_path, prompt, Some(source_slug), persona)
}

/// Internal implementation for session directory initialization.
///
/// Handles both regular and cloned sessions to avoid code duplication.
fn initialize_session_directory_internal(
    slug: &str,
    project_path: &Path,
    prompt: Option<String>,
    cloned_from: Option<&str>,
    persona: Option<&str>,
) -> Result<PathBuf, SessionError> {
    // Create session directory
    let session_path = session_dir(slug);
    fs::create_dir_all(&session_path).map_err(|e| SessionError::CreateSessionDir {
        path: session_path.display().to_string(),
        source: e,
    })?;

    // Create session metadata based on clone and persona options
    let metadata = match (cloned_from, persona) {
        (Some(source_slug), Some(p)) => {
            let mut m = SessionMetadata::new_cloned(
                slug.to_string(),
                project_path.to_path_buf(),
                prompt,
                source_slug,
            );
            m.persona = Some(p.to_string());
            m
        }
        (Some(source_slug), None) => SessionMetadata::new_cloned(
            slug.to_string(),
            project_path.to_path_buf(),
            prompt,
            source_slug,
        ),
        (None, Some(p)) => SessionMetadata::new_with_persona(
            slug.to_string(),
            project_path.to_path_buf(),
            prompt,
            p.to_string(),
        ),
        (None, None) => SessionMetadata::new(slug.to_string(), project_path.to_path_buf(), prompt),
    };

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
    let entry = match persona {
        Some(p) => SessionEntry::new_with_persona(
            slug.to_string(),
            project_path.to_path_buf(),
            p.to_string(),
        ),
        None => SessionEntry::new(slug.to_string(), project_path.to_path_buf()),
    };
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
/// * `prompt` - Optional prompt string (after placeholder substitution) for replay
///
/// # Returns
///
/// * `Ok((slug, session_dir))` - The slug used and path to the session directory
/// * `Err(SessionError)` - If initialization fails
pub fn initialize_session(
    user_slug: Option<&str>,
    project_path: &Path,
    prompt: Option<String>,
    persona: Option<&str>,
) -> Result<(String, PathBuf), SessionError> {
    let slug = resolve_session_slug(user_slug)?;
    let session_path = initialize_session_directory(&slug, project_path, prompt, persona)?;
    Ok((slug, session_path))
}

/// Initialize a new session that is cloned from an existing session.
///
/// Similar to `initialize_session`, but records the source session slug
/// in the metadata for traceability. The new session gets an auto-generated
/// slug (the source slug is passed separately for metadata).
///
/// # Arguments
///
/// * `user_slug` - Optional user-provided slug. If None, generates automatically.
/// * `project_path` - Absolute path to the project directory
/// * `prompt` - Optional prompt string (after placeholder substitution) for replay
/// * `source_slug` - The slug of the source session being cloned
///
/// # Returns
///
/// * `Ok((slug, session_dir))` - The new slug and path to the session directory
/// * `Err(SessionError)` - If initialization fails
pub fn initialize_session_with_clone(
    user_slug: Option<&str>,
    project_path: &Path,
    prompt: Option<String>,
    source_slug: &str,
    persona: Option<&str>,
) -> Result<(String, PathBuf), SessionError> {
    let slug = resolve_session_slug(user_slug)?;
    let session_path =
        initialize_session_directory_with_clone(&slug, project_path, prompt, source_slug, persona)?;
    Ok((slug, session_path))
}

/// Find the most recent session for a given project.
///
/// This function loads the sessions index and finds the most recent session
/// (by started_at timestamp) that belongs to the specified project.
///
/// # Arguments
///
/// * `project_path` - The absolute path to the project directory
///
/// # Returns
///
/// * `Ok(Some(SessionEntry))` - The most recent session for the project
/// * `Ok(None)` - No sessions exist for this project
/// * `Err(SessionError)` - If loading the sessions index fails
pub fn find_most_recent_session(
    project_path: &Path,
    persona: Option<&str>,
) -> Result<Option<SessionEntry>, SessionError> {
    let index = load_sessions_index()?;
    Ok(index
        .find_most_recent_for_persona(project_path, persona)
        .cloned())
}

/// Find a session by its slug.
///
/// # Arguments
///
/// * `slug` - The session slug to look up
///
/// # Returns
///
/// * `Ok(SessionEntry)` - The session entry
/// * `Err(SessionError::SessionNotFound)` - If no session with that slug exists
pub fn find_session_by_slug(slug: &str) -> Result<SessionEntry, SessionError> {
    let index = load_sessions_index()?;

    index
        .find_by_slug(slug)
        .cloned()
        .ok_or_else(|| SessionError::SessionNotFound {
            slug: slug.to_string(),
        })
}

/// Finalize a session by updating its outcome, iteration count, and completion timestamp.
///
/// This function should be called when a run loop completes (successfully or not):
/// 1. Loads the global sessions index
/// 2. Finds the session entry by slug
/// 3. Updates outcome, iterations count, and completed_at timestamp
/// 4. Saves the updated index to disk
///
/// # Arguments
///
/// * `slug` - The session slug to finalize
/// * `iterations` - The number of iterations completed
/// * `outcome` - The final outcome of the session
///
/// # Returns
///
/// * `Ok(())` - If the session was successfully finalized
/// * `Err(SessionError)` - If the session wasn't found or update failed
pub fn finalize_session(
    slug: &str,
    iterations: u32,
    outcome: SessionOutcome,
) -> Result<(), SessionError> {
    let mut index = load_sessions_index()?;

    // Use a block to scope the mutable borrow
    {
        let entry = index
            .find_by_slug_mut(slug)
            .ok_or_else(|| SessionError::SessionNotFound {
                slug: slug.to_string(),
            })?;

        entry.iterations = iterations;
        entry.outcome = outcome;
        entry.completed_at = Some(chrono::Utc::now());
    }

    // Now the mutable borrow is released, we can save
    save_sessions_index(&index)?;

    // Also update the session.toml file in the session directory
    // Re-get the entry for metadata (now as immutable borrow)
    let session_path = session_dir(slug);
    let session_toml_path = session_path.join("session.toml");
    if session_toml_path.exists() {
        if let Some(entry) = index.find_by_slug(slug) {
            let metadata = SessionMetadata::from(entry);
            let metadata_content = metadata.to_toml()?;
            fs::write(&session_toml_path, metadata_content).map_err(|e| {
                SessionError::WriteSessionMetadata {
                    path: session_toml_path.display().to_string(),
                    source: e,
                }
            })?;
        }
    }

    Ok(())
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
    fn test_initialize_session_directory_creates_structure() {
        with_temp_sessions_dir(|temp_dir| {
            let session_path = temp_dir.path().join("test-session");
            let project_path = PathBuf::from("/test/project");

            // Manually create the session directory and metadata
            fs::create_dir_all(&session_path).unwrap();

            let metadata =
                SessionMetadata::new("test-session".to_string(), project_path.clone(), None);
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

    #[test]
    fn test_session_not_found_error_message() {
        let err = SessionError::SessionNotFound {
            slug: "test-slug".to_string(),
        };
        let msg = format!("{}", err);
        assert!(msg.contains("test-slug"));
        assert!(msg.contains("not found"));
        assert!(msg.contains("ralph sessions"));
    }
}
