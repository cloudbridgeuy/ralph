//! Warning message utilities (Imperative Shell).
//!
//! This module provides centralized helpers for consistent warning output.
//! All warning messages follow the format "Warning: {context}: {error}" or
//! "Warning: {message}." for non-error warnings.

use std::fmt::Display;

/// Print a warning message to stderr.
///
/// Outputs in the format "Warning: {message}".
///
/// # Examples
///
/// ```no_run
/// use ralph::warn::warn;
///
/// warn("Not a git repository. Skipping diff capture.");
/// // Output: Warning: Not a git repository. Skipping diff capture.
/// ```
pub fn warn(message: impl Display) {
    eprintln!("Warning: {}", message);
}

/// Log a warning if the result is an error, then discard the result.
///
/// Outputs in the format "Warning: {context}: {error}".
///
/// # Arguments
///
/// * `result` - The Result to check
/// * `context` - A description of what operation failed
///
/// # Examples
///
/// ```no_run
/// use ralph::warn::warn_if_err;
/// use std::io;
///
/// fn may_fail() -> Result<(), io::Error> {
///     Err(io::Error::new(io::ErrorKind::NotFound, "file not found"))
/// }
///
/// warn_if_err(may_fail(), "Failed to finalize session");
/// // Output: Warning: Failed to finalize session: file not found
/// ```
pub fn warn_if_err<T, E: Display>(result: Result<T, E>, context: impl Display) {
    if let Err(e) = result {
        eprintln!("Warning: {}: {}", context, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_warn_if_err_with_ok_does_not_panic() {
        let result: Result<i32, io::Error> = Ok(42);
        warn_if_err(result, "This should not print");
        // Test passes if no panic occurs
    }

    #[test]
    fn test_warn_if_err_with_err_does_not_panic() {
        let result: Result<i32, io::Error> =
            Err(io::Error::new(io::ErrorKind::NotFound, "test error"));
        warn_if_err(result, "Test context");
        // Test passes if no panic occurs
    }
}
