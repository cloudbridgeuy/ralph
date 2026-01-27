//! Core types for subprocess operations.

use crate::highlight::ThemeError;
use crate::stream_processor::StreamProcessorResult;
use std::io;

/// Default gap threshold before showing spinner (in milliseconds).
/// If no output is received for this duration, spinner reappears.
pub const DEFAULT_GAP_THRESHOLD_MS: u64 = 500;

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
    /// Whether a soft stop was requested during execution.
    /// When true, the iteration should complete but the run loop should pause afterward.
    pub soft_stop_requested: bool,
}

impl StreamingSubprocessResult {
    /// Returns true if a soft stop was requested during execution.
    pub fn is_soft_stop_requested(&self) -> bool {
        self.soft_stop_requested
    }
}
