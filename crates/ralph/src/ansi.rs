//! ANSI escape sequences for terminal control.
//!
//! This module provides common ANSI escape codes used throughout the application
//! for terminal manipulation and text styling.

/// Carriage return - moves cursor to beginning of line.
pub const CR: &str = "\r";

/// Clear from cursor to end of line.
pub const CLEAR_EOL: &str = "\x1b[K";

/// Set foreground color to cyan.
pub const CYAN: &str = "\x1b[36m";

/// Set foreground color to yellow.
pub const YELLOW: &str = "\x1b[33m";

/// Set foreground color to green.
pub const GREEN: &str = "\x1b[32m";

/// Set text to dim.
pub const DIM: &str = "\x1b[2m";

/// Reset all text formatting.
pub const RESET: &str = "\x1b[0m";
