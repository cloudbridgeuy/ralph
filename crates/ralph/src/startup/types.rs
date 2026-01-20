//! Data structures for startup and iteration display.

use std::path::PathBuf;

/// Version of the ralph binary (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Information to display at startup.
#[derive(Debug)]
pub struct StartupInfo {
    /// The session slug (generated or user-provided).
    pub slug: String,
    /// Total number of stories in the PRD.
    pub total_stories: usize,
    /// Number of pending stories.
    pub pending_stories: usize,
    /// Number of completed stories.
    pub completed_stories: usize,
    /// Maximum iterations to run.
    pub max_iterations: usize,
    /// Whether max_iterations was explicitly provided by user.
    pub iterations_from_arg: bool,
    /// Path to PRD file (only shown if custom).
    pub custom_prd_path: Option<PathBuf>,
    /// Path to design file (only shown if custom).
    pub custom_design_path: Option<PathBuf>,
    /// Path to progress file (only shown if custom).
    pub custom_progress_path: Option<PathBuf>,
    /// Whether a custom command template is used.
    pub custom_command: bool,
    /// Whether a custom prompt is used.
    pub custom_prompt: bool,
    /// Whether a custom completion marker is used.
    pub custom_completion_marker: bool,
    /// Whether an additional prompt was provided.
    pub custom_additional_prompt: bool,
    /// Session directory path.
    pub session_dir: PathBuf,
}

impl StartupInfo {
    /// Check if any custom configuration is being used.
    pub(super) fn has_custom_config(&self) -> bool {
        self.custom_prd_path.is_some()
            || self.custom_design_path.is_some()
            || self.custom_progress_path.is_some()
            || self.custom_command
            || self.custom_prompt
            || self.custom_completion_marker
            || self.custom_additional_prompt
    }
}

/// Information for iteration header display.
#[derive(Debug)]
pub struct IterationHeader {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Maximum iterations, if known.
    pub max_iterations: Option<usize>,
    /// Number of pending stories at the start of this iteration.
    pub pending_stories: usize,
}

/// Information for iteration summary display.
#[derive(Debug)]
pub struct IterationSummary {
    /// Current iteration number (1-indexed).
    pub iteration: usize,
    /// Cost in USD for this iteration (from result event).
    pub cost_usd: Option<f64>,
    /// Duration in milliseconds (from result event).
    pub duration_ms: Option<u64>,
    /// Model name used for this iteration.
    pub model: Option<String>,
    /// Input tokens used.
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    pub output_tokens: Option<u64>,
}

/// Information for final run summary display.
#[derive(Debug, Clone)]
pub struct RunSummary {
    /// The session slug.
    pub slug: String,
    /// Total number of iterations completed.
    pub iterations_completed: usize,
    /// Reason for completion (if any).
    pub completion_reason: Option<String>,
    /// Total cost across all iterations (USD).
    pub total_cost_usd: Option<f64>,
    /// Total duration across all iterations (wall clock time from start).
    pub total_duration_ms: Option<u64>,
    /// Total input tokens across all iterations.
    pub total_input_tokens: Option<u64>,
    /// Total output tokens across all iterations.
    pub total_output_tokens: Option<u64>,
    /// Final pending story count.
    pub final_pending_stories: usize,
}

/// Information for prompt display before iterations begin.
#[derive(Debug, Clone)]
pub struct PromptDisplay<'a> {
    /// The prompt text to display.
    pub prompt: &'a str,
}
