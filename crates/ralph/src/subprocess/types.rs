//! Core types for subprocess operations.

use crate::highlight::ThemeError;
use crate::stream_processor::StreamProcessorResult;
use std::io;

/// Default gap threshold before showing spinner (in milliseconds).
/// If no output is received for this duration, spinner reappears.
pub const DEFAULT_GAP_THRESHOLD_MS: u64 = 500;

/// Exit code indicating subprocess was killed due to timeout.
pub const EXIT_CODE_KILLED: i32 = -1;

/// Exit code indicating subprocess was interrupted by signal (SIGINT/SIGTERM).
pub const EXIT_CODE_INTERRUPTED: i32 = -2;

/// Error type for subprocess operations.
#[derive(Debug, thiserror::Error)]
pub enum SubprocessError {
    #[error("Failed to spawn subprocess: {0}")]
    SpawnFailed(#[from] io::Error),

    #[error("Subprocess terminated by signal")]
    Signaled,

    #[error("Failed to capture output: {0}")]
    OutputCaptureFailed(String),

    #[error("Subprocess timed out after {timeout_secs} seconds")]
    Timeout {
        /// Timeout duration in seconds
        timeout_secs: u64,
        /// Partial output captured before timeout
        partial_result: Box<StreamingSubprocessResult>,
    },

    #[error("Subprocess interrupted by SIGINT/SIGTERM")]
    Interrupted {
        /// Partial output captured before interrupt
        partial_result: Box<StreamingSubprocessResult>,
    },

    #[error("Subprocess hard-stopped by user (S key)")]
    HardStop {
        /// Partial output captured before hard stop
        partial_result: Box<StreamingSubprocessResult>,
    },

    #[error("Invalid theme configuration: {0}")]
    ThemeError(#[from] ThemeError),
}

/// Result of a subprocess invocation with stream processing.
#[derive(Debug)]
pub struct StreamingSubprocessResult {
    /// The exit code from the subprocess.
    pub exit_code: i32,
    /// Captured stderr output.
    pub stderr: String,
    /// Processed stream result with chunks, metadata, and tool interactions.
    pub stream_result: StreamProcessorResult,
}
