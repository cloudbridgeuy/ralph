//! Error types for iteration log operations.

/// Error type for iteration log operations.
#[derive(Debug, thiserror::Error)]
pub enum IterationError {
    /// Failed to write iteration log
    #[error("Failed to write iteration log at {path}: {source}")]
    WriteLog {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to serialize iteration log
    #[error("Failed to serialize iteration log: {0}")]
    SerializeLog(#[from] toml::ser::Error),
}
