//! CLI argument parsing for ralph.
//!
//! This module defines the command-line interface using clap derive macros.
//! It provides the main CLI struct and all subcommand definitions.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

/// Ralph - LLM-driven development workflow automation.
///
/// Ralph implements an iterative LLM-driven development workflow. Work is divided
/// into discrete user stories defined in a PRD. Each iteration spawns a fresh LLM
/// session that picks up one story, implements it, updates tracking files, and commits.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)] // RunArgs is large but this is CLI parsing, not hot path
pub enum Commands {
    /// Run the iteration loop to process user stories.
    ///
    /// Spawns LLM sessions iteratively to implement pending user stories from
    /// the PRD. Each iteration picks up one story, implements it, updates the
    /// PRD to mark it complete, and commits the changes.
    Run(RunArgs),

    /// List all sessions across all projects.
    ///
    /// Displays a table of sessions with their slug, project path,
    /// start date, iteration count, and outcome status.
    Sessions(SessionsArgs),

    /// List all iterations across all sessions.
    ///
    /// Displays a table of iterations with session, sequence number,
    /// project path, timestamp, duration, exit code, and optionally cost.
    Iterations(IterationsArgs),

    /// Replay a session's output with syntax highlighting.
    ///
    /// Re-renders the captured output from a previous session, applying
    /// syntax highlighting to code blocks and diff highlighting to diffs.
    /// Works with sessions from any project.
    Replay(ReplayArgs),

    /// List available syntax highlighting themes.
    ///
    /// Shows all built-in themes that can be used with the --theme flag.
    /// Custom .tmTheme files can also be loaded by specifying a file path.
    Themes,

    /// Send a single-shot prompt to the LLM and display the response.
    ///
    /// A simplified interface for quick LLM interactions. Each invocation
    /// creates a session that can be replayed or continued later.
    Ask(AskArgs),
}

/// Arguments for the `sessions` subcommand.
#[derive(clap::Args, Debug)]
pub struct SessionsArgs {
    /// Filter sessions by project path (substring match).
    #[arg(long)]
    pub project: Option<String>,

    /// Filter sessions by outcome status.
    ///
    /// Valid values: in_progress, completed, aborted, failed, interrupted
    #[arg(long)]
    pub outcome: Option<String>,
}

/// Arguments for the `iterations` subcommand.
#[derive(clap::Args, Debug)]
pub struct IterationsArgs {
    /// Filter iterations by session slug.
    #[arg(long)]
    pub session: Option<String>,

    /// Filter iterations by project path (substring match).
    #[arg(long)]
    pub project: Option<String>,

    /// Filter iterations by outcome.
    ///
    /// Valid values: completed (exit_code 0), failed (non-zero exit_code)
    #[arg(long)]
    pub outcome: Option<String>,
}

/// Arguments for the `replay` subcommand.
#[derive(clap::Args, Debug)]
pub struct ReplayArgs {
    /// Session identifier (slug) to replay.
    ///
    /// The slug uniquely identifies a session. Use 'ralph sessions'
    /// to list available sessions.
    #[arg(value_name = "SLUG")]
    pub slug: String,

    /// Only replay a specific iteration (1-indexed).
    ///
    /// If omitted, all iterations are replayed in order.
    #[arg(short, long, value_name = "N")]
    pub iteration: Option<u32>,

    /// Syntax highlighting theme.
    ///
    /// Use a built-in theme name (e.g., "Monokai Extended", "Solarized (dark)")
    /// or a path to a custom .tmTheme file. Run 'ralph themes' to list available themes.
    /// Can also be set via RALPH_THEME environment variable or config file.
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// Disable background colors in syntax highlighting.
    ///
    /// When set, theme background colors are not applied, allowing the
    /// terminal's default background to show through.
    /// Can also be set via RALPH_NO_BACKGROUND environment variable or config file.
    #[arg(long)]
    pub no_background: bool,

    /// Suppress prompt display at the start of replay.
    ///
    /// By default, the stored prompt is displayed before Iteration 1.
    /// Use this flag to hide the prompt.
    #[arg(long)]
    pub no_prompt: bool,

    /// Pause duration in seconds between each output block.
    ///
    /// When specified, replay pauses for the given duration after rendering
    /// each output block. Supports fractional seconds (e.g., 0.5, 1.5).
    /// When omitted, blocks render immediately with no delay.
    #[arg(long, value_name = "SECONDS")]
    pub delay: Option<f64>,
}

/// Arguments for the `ask` subcommand.
#[derive(clap::Args, Debug, Clone)]
pub struct AskArgs {
    // --- Prompt ---
    /// The prompt to send to the LLM.
    ///
    /// If not provided, reads from stdin. Supports inline text or "-" for stdin.
    #[arg(value_name = "PROMPT")]
    pub prompt: Option<String>,

    // --- Session ---
    /// Session name for the new session, or session to continue with --continue.
    ///
    /// Used to name the session directory for logs.
    /// Auto-generated as adjective-noun (e.g., "quiet-mountain") if omitted.
    /// Must be lowercase with exactly one hyphen (e.g., "my-session").
    /// When used with --continue, specifies which session to continue.
    #[arg(short = 'S', long)]
    pub session: Option<String>,

    /// Continue an existing session instead of creating a new one.
    ///
    /// When used alone, continues the most recent session for the current project.
    /// When used with --session, continues the specified session.
    /// Adds a new iteration to the session with the next sequence number.
    #[arg(short = 'c', long = "continue")]
    pub continue_session: bool,

    // --- Theme ---
    /// Syntax highlighting theme.
    ///
    /// Use a built-in theme name (e.g., "Monokai Extended", "Solarized (dark)")
    /// or a path to a custom .tmTheme file. Run 'ralph themes' to list available themes.
    /// Can also be set via RALPH_THEME environment variable or config file.
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// Disable background colors in syntax highlighting.
    ///
    /// When set, theme background colors are not applied, allowing the
    /// terminal's default background to show through.
    /// Can also be set via RALPH_NO_BACKGROUND environment variable or config file.
    #[arg(long)]
    pub no_background: bool,

    // --- Execution ---
    /// Timeout for LLM subprocess in seconds.
    ///
    /// If the subprocess exceeds this duration, it is killed and treated
    /// as a failure. Prevents runaway processes.
    /// Default: 600 seconds (10 minutes)
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,

    /// Enable verbose output for specific tools.
    ///
    /// Accepts a comma-separated list of tool names (case-insensitive).
    /// When verbose is enabled for a tool, full input/output is shown
    /// instead of truncated summaries.
    ///
    /// Examples:
    ///   --verbose-tools              Enable verbose for all tools
    ///   --verbose-tools=grep,bash    Enable for Grep and Bash only
    ///   --verbose-tools=read         Enable for Read only
    #[arg(long, value_name = "TOOLS", num_args = 0..=1, default_missing_value = "*")]
    pub verbose_tools: Option<String>,

    /// Suppress prompt display at the start of execution.
    ///
    /// By default, the prompt passed to the LLM is displayed before
    /// execution begins. Use this flag to hide the prompt.
    #[arg(long)]
    pub no_prompt: bool,

    /// Display conversation history for the session.
    ///
    /// Shows all previous user prompts and assistant responses in chronological order.
    /// Requires --session or --continue to specify which session to display history for.
    /// If no prompt argument is provided, displays history and exits.
    /// If a prompt argument is provided with --continue, displays history then proceeds.
    #[arg(long)]
    pub history: bool,

    /// Permission mode for tool execution.
    ///
    /// Controls how Claude handles tool execution permissions:
    /// - default: Requires approval for all tools
    /// - acceptEdits: Auto-accepts file edits but requires approval for other tools
    /// - plan: Read-only mode, no tools can modify files
    /// - bypassPermissions: Auto-accepts all tool executions (default for ask command)
    ///
    /// SECURITY NOTE: bypassPermissions allows Claude to execute any tool without
    /// confirmation. Use with caution in untrusted environments.
    #[arg(long, value_name = "MODE")]
    pub permission_mode: Option<String>,

    /// Clone an existing session into a new session with its conversation history.
    ///
    /// Creates a new session pre-populated with the conversation history from the
    /// source session. The original session remains unchanged. Use with --session
    /// to specify the source session, or with --continue to clone from the most
    /// recent session. The new session gets an auto-generated name.
    ///
    /// Example:
    ///   ralph ask --session my-test --clone-session 'new direction'
    ///   ralph ask --continue --clone-session 'branch question'
    #[arg(long)]
    pub clone_session: bool,
}

/// Arguments for the `run` subcommand.
#[derive(clap::Args, Debug)]
pub struct RunArgs {
    /// Maximum number of iterations to run.
    ///
    /// Defaults to the number of pending stories in the PRD.
    /// The loop exits early if all stories are completed or
    /// the completion marker is found in output.
    #[arg(value_name = "ITERATIONS")]
    pub iterations: Option<usize>,

    /// Session identifier.
    ///
    /// Used to name the session directory for logs.
    /// Auto-generated as adjective-noun (e.g., "quiet-mountain") if omitted.
    #[arg(short, long)]
    pub slug: Option<String>,

    /// Custom prompt template.
    ///
    /// Supports file path, `-` for stdin, or inline string.
    /// Placeholders: {prd_file}, {completion_marker}, {additional_prompt}
    #[arg(short, long)]
    pub prompt: Option<String>,

    /// Custom LLM invocation pattern.
    ///
    /// Command template to execute with {prompt} placeholder.
    /// Default: claude --permission-mode acceptEdits --output-format stream-json -p {prompt}
    #[arg(short, long, value_name = "TEMPLATE")]
    pub command: Option<String>,

    /// PRD file path.
    ///
    /// Default: .local/plans/prd.toml
    #[arg(long, value_name = "PATH")]
    pub prd: Option<PathBuf>,

    /// Maximum failure recovery attempts.
    ///
    /// Number of times to automatically re-attempt if the LLM subprocess fails.
    /// After exhausting all attempts, prompts user for action.
    #[arg(long, default_value_t = 3)]
    pub max_attempts: usize,

    /// Custom completion marker.
    ///
    /// When found in LLM output, exits the loop immediately.
    /// Default: <promise>COMPLETE</promise>
    #[arg(long, value_name = "STRING")]
    pub completion_marker: Option<String>,

    /// Timeout for LLM subprocess in seconds.
    ///
    /// If the subprocess exceeds this duration, it is killed and treated
    /// as a failure (retry logic applies). Prevents runaway processes.
    /// Default: 600 seconds (10 minutes)
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,

    /// Syntax highlighting theme.
    ///
    /// Use a built-in theme name (e.g., "Monokai Extended", "Solarized (dark)")
    /// or a path to a custom .tmTheme file. Run 'ralph themes' to list available themes.
    /// Can also be set via RALPH_THEME environment variable.
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// Disable background colors in syntax highlighting.
    ///
    /// When set, theme background colors are not applied, allowing the
    /// terminal's default background to show through.
    /// Can also be set via RALPH_NO_BACKGROUND environment variable.
    #[arg(long)]
    pub no_background: bool,

    /// Additional instructions to append to the prompt.
    ///
    /// Appends custom instructions to the main prompt template.
    /// Supports file path, `-` for stdin, or inline string.
    /// Useful for adding project-specific or one-off instructions.
    #[arg(short = 'a', long)]
    pub additional_prompt: Option<String>,

    /// Enable verbose output for specific tools.
    ///
    /// Accepts a comma-separated list of tool names (case-insensitive).
    /// When verbose is enabled for a tool, full input/output is shown
    /// instead of truncated summaries.
    ///
    /// Examples:
    ///   --verbose-tools              Enable verbose for all tools
    ///   --verbose-tools=grep,bash    Enable for Grep and Bash only
    ///   --verbose-tools=read         Enable for Read only
    #[arg(long, value_name = "TOOLS", num_args = 0..=1, default_missing_value = "*")]
    pub verbose_tools: Option<String>,

    /// Suppress prompt display at the start of each run.
    ///
    /// By default, the prompt passed to the LLM is displayed before
    /// Iteration 1 begins. Use this flag to hide the prompt.
    #[arg(long)]
    pub no_prompt: bool,
}
