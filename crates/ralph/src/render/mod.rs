//! Shared rendering module for OutputBlock display.
//!
//! This module provides unified rendering utilities used by both live execution
//! (stream processor) and replay. By consolidating these functions, we ensure
//! consistent output formatting across both rendering paths.
//!
//! # Module Structure
//!
//! - `utils`: Pure utility functions for line normalization, file grouping, etc.
//!
//! # Design Principles
//!
//! 1. **Pure Functions**: All utilities are stateless and free of side effects
//! 2. **Shared Logic**: Functions used by both stream processor and replay
//! 3. **Single Source of Truth**: No duplication between rendering paths

pub mod utils;

// Re-export commonly used functions for convenience
pub use utils::{
    extract_language_from_path, extract_line_number, group_files_by_directory,
    highlight_grep_match, normalize_cat_n_format,
};
