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

    #[error("claude process exited before the turn")]
    Stale,

    #[error("claude process wrote a result after the last turn")]
    LeftoverResult,

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

impl SubprocessError {
    pub fn partial_result(&self) -> Option<&StreamingSubprocessResult> {
        match self {
            SubprocessError::Timeout { partial_result, .. }
            | SubprocessError::Interrupted { partial_result } => Some(partial_result),
            _ => None,
        }
    }
}

pub fn recorded(
    outcome: &Result<StreamingSubprocessResult, SubprocessError>,
) -> Option<&StreamingSubprocessResult> {
    match outcome {
        Ok(done) => Some(done),
        Err(error) => error.partial_result(),
    }
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
    pub request_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESULT_LINE: &str = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.0024,"duration_ms":4404,"session_id":"s1","usage":{"input_tokens":2,"output_tokens":774}}"#;

    fn partial(line: Option<&str>) -> StreamingSubprocessResult {
        let mut processor = crate::stream_processor::StreamProcessor::new();
        if let Some(line) = line {
            processor.process_line(line);
        }
        StreamingSubprocessResult {
            exit_code: EXIT_CODE_KILLED,
            stderr: String::new(),
            stream_result: processor.finish(),
            request_at: None,
        }
    }

    fn output_tokens(outcome: &Result<StreamingSubprocessResult, SubprocessError>) -> Option<u64> {
        recorded(outcome)
            .and_then(|done| done.stream_result.costs.usage.as_ref())
            .map(|usage| usage.output_tokens)
    }

    #[test]
    fn recorded_keeps_the_usage_of_a_timed_out_call() {
        let outcome = Err(SubprocessError::Timeout {
            timeout_secs: 1,
            partial_result: Box::new(partial(Some(RESULT_LINE))),
        });
        assert_eq!(output_tokens(&outcome), Some(774));
    }

    #[test]
    fn recorded_keeps_the_usage_of_an_interrupted_call() {
        let outcome = Err(SubprocessError::Interrupted {
            partial_result: Box::new(partial(Some(RESULT_LINE))),
        });
        assert_eq!(output_tokens(&outcome), Some(774));
    }

    #[test]
    fn recorded_keeps_a_finished_call() {
        assert_eq!(output_tokens(&Ok(partial(Some(RESULT_LINE)))), Some(774));
    }

    #[test]
    fn recorded_has_no_usage_when_no_result_event_arrived() {
        let outcome = Err(SubprocessError::Interrupted {
            partial_result: Box::new(partial(None)),
        });
        assert!(recorded(&outcome).is_some());
        assert_eq!(output_tokens(&outcome), None);
    }

    #[test]
    fn recorded_drops_a_call_that_never_ran() {
        assert!(recorded(&Err(SubprocessError::Stale)).is_none());
        let spawn = SubprocessError::SpawnFailed(io::Error::other("no claude"));
        assert!(recorded(&Err(spawn)).is_none());
    }
}
