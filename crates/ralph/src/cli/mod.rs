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
    /// List available syntax highlighting themes.
    ///
    /// Shows all built-in themes that can be used with the --theme flag.
    /// Custom .tmTheme files can also be loaded by specifying a file path.
    Themes,

    /// Send a single-shot prompt to the LLM and display the response.
    ///
    /// A simplified interface for quick LLM interactions. Nothing is stored.
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

    #[command(hide = true)]
    WorkerStopHook(WorkerStopHookArgs),
}

#[derive(clap::Args, Debug)]
pub struct ChatArgs {
    #[arg(long, value_name = "DIR", hide = true)]
    pub store: Option<std::path::PathBuf>,

    #[arg(long, default_value_t = CHAT_TIMEOUT_SECS)]
    pub timeout: u64,

    #[arg(long)]
    pub frozen_window: Option<usize>,

    #[arg(long = "skill", value_name = "PATH")]
    pub skills: Vec<std::path::PathBuf>,

    #[arg(long, value_name = "MODE")]
    pub permission_mode: Option<String>,

    #[arg(long, value_name = "DIR")]
    pub worktree_dir: Option<String>,

    #[arg(long)]
    pub no_inbox_sound: bool,

    #[arg(long)]
    pub no_inbox_notification: bool,
}

#[derive(clap::Args, Debug)]
pub struct ChatMcpArgs {
    #[arg(long, value_name = "DIR")]
    pub store: std::path::PathBuf,

    #[arg(long, value_name = "MODE", default_value = crate::invoke::CHAT_DEFAULT_PERMISSION_MODE)]
    pub permission_mode: String,

    #[arg(long, value_name = "DIR")]
    pub worktree_dir: Option<String>,
}

#[derive(clap::Args, Debug)]
pub struct WorkerMcpArgs {
    #[arg(long, value_name = "WORKER")]
    pub id: String,

    #[arg(long, value_name = "DIR")]
    pub inbox: std::path::PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct WorkerStopHookArgs {
    #[arg(long, value_name = "PATH")]
    pub reported: std::path::PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct ChatNapArgs {
    #[arg(long, value_name = "DIR")]
    pub store: std::path::PathBuf,

    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
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

    /// Permission mode for tool execution.
    ///
    /// Controls how Claude handles tool execution permissions:
    /// - default: Requires approval for all tools
    /// - acceptEdits: Auto-accepts file edits but requires approval for other tools
    /// - plan: Read-only mode, no tools can modify files
    /// - auto: Claude decides which tool executions to approve
    /// - bypassPermissions: Auto-accepts all tool executions (default for ask command)
    ///
    /// SECURITY NOTE: bypassPermissions allows Claude to execute any tool without
    /// confirmation. Use with caution in untrusted environments.
    #[arg(long, value_name = "MODE")]
    pub permission_mode: Option<String>,
}
