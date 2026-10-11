//! CLI argument parsing for ralph.
//!
//! This module defines the command-line interface using clap derive macros.
//! It provides the main CLI struct and all subcommand definitions.

use clap::{Parser, Subcommand};

#[cfg(test)]
mod tests;

pub const CHAT_TIMEOUT_SECS: u64 = 600;

/// Ralph - chat and single-shot Claude sessions with rich terminal rendering.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List all sessions across all projects.
    ///
    /// Displays a table of sessions with their slug, project path,
    /// start date, iteration count, and outcome status.
    Sessions(SessionsArgs),

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

    /// Chat with a persistent, compressed memory.
    ///
    /// Runs when no subcommand is given. Each message is a fresh Claude turn
    /// over the whole chat so far, stored under ~/.ralph/chat.
    Chat(ChatArgs),

    #[command(hide = true)]
    ChatMcp(ChatMcpArgs),

    #[command(hide = true)]
    ChatNap(ChatNapArgs),

    #[command(hide = true)]
    WorkerMcp(WorkerMcpArgs),
}

#[derive(clap::Args, Debug)]
pub struct ChatArgs {
    #[arg(long, value_name = "DIR", hide = true)]
    pub store: Option<std::path::PathBuf>,

    #[arg(long, default_value_t = CHAT_TIMEOUT_SECS)]
    pub timeout: u64,

    #[arg(long = "skill", value_name = "PATH")]
    pub skills: Vec<std::path::PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct ChatMcpArgs {
    #[arg(long, value_name = "DIR")]
    pub store: std::path::PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct WorkerMcpArgs {
    #[arg(long, value_name = "WORKER")]
    pub id: String,

    #[arg(long, value_name = "DIR")]
    pub inbox: std::path::PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct ChatNapArgs {
    #[arg(long, value_name = "DIR")]
    pub store: std::path::PathBuf,

    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
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
