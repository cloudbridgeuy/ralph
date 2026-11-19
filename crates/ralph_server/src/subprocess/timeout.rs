use super::types::SubprocessError;
use std::process::{Child, ExitStatus};

/// Helper to try waiting for child process without blocking.
pub(super) fn try_wait_child(child: &mut Child) -> Result<Option<ExitStatus>, SubprocessError> {
    child.try_wait().map_err(|e| {
        SubprocessError::OutputCaptureFailed(format!("Failed to check subprocess status: {}", e))
    })
}
