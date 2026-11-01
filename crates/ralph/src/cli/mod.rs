//! CLI argument parsing for ralph.
//!
//! This module defines the command-line interface using clap derive macros.
//! It provides the main CLI struct and all subcommand definitions.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[cfg(test)]
mod tests;

pub const NAP_TIMEOUT_SECS: u64 = 600;

/// Chat with Claude from the terminal: persistent memory, background workers, rich rendering.
///
/// Bare `ralph` runs `ralph chat`: one long-running chat whose memory is a
/// compressed tree of summaries, kept small by a background nap pass. From
/// chat, Claude can start workers (claude in tmux, Cursor, or opencode), each
/// in its own git worktree, and watch long-running commands. Their reports
/// land in the chat inbox.
///
/// `ralph ask` sends one prompt and stores nothing. `ralph themes` lists the
/// syntax highlighting themes.
#[derive(Parser, Debug)]
#[command(author, version = crate::version::current())]
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
    /// Runs when no subcommand is given. One Claude process serves the chat,
    /// started with a compressed view of the chat so far, stored under
    /// ~/.ralph/chat.
    Chat(ChatArgs),

    #[command(
        about = "Record the wire traffic of an LLM harness through a local proxy.",
        long_about = "Starts a local HTTP proxy that forwards every request to the provider \
unchanged and records one line per call under a session directory. Prompts are \
stored in plaintext. Export the printed base URL variable before starting the \
harness; ralph never sets it."
    )]
    Wire(WireArgs),

    #[command(
        about = "Inspect ralph's config file.",
        long_about = "Subcommands inspect config.toml without starting a chat."
    )]
    Config(ConfigArgs),

    #[command(
        about = "Check a headless worker agent against ralph's local spec.",
        long_about = "Subcommands exercise the installed headless agent without starting a chat."
    )]
    Worker(WorkerArgs),

    #[command(hide = true)]
    ChatMcp(ChatMcpArgs),

    #[command(hide = true)]
    ChatNap(ChatNapArgs),

    #[command(hide = true, name = "__mermaid")]
    Mermaid(MermaidArgs),

    #[command(hide = true)]
    WorkerMcp(WorkerMcpArgs),

    #[command(hide = true)]
    WorkerStopHook(WorkerStopHookArgs),
}

#[derive(clap::Args, Debug)]
pub struct WorkerArgs {
    #[command(subcommand)]
    pub command: WorkerCommand,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyAgent {
    Cursor,
    Codex,
}

impl VerifyAgent {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Codex => "codex",
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum WorkerCommand {
    #[command(
        about = "Run the local spec against the installed agent and record a passing version.",
        long_about = "Runs the agent in a fresh temporary directory and prints PASS or FAIL per \
check. Exits non-zero when any blocking check fails. The free spec checks the version, handshake, \
login and session setup without sending a prompt. --live adds one prompt, one permission \
round-trip and one cancel, which use model calls; a failed or unknown cancel never fails the \
spec. A full pass records the version in verified-agents.json, which silences the version \
warning for that version."
    )]
    Verify {
        #[arg(value_enum, help = "Agent to verify")]
        agent: VerifyAgent,
        #[arg(long, help = "Also run the checks that send prompts to the model")]
        live: bool,
    },
}

#[derive(clap::Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    #[command(
        about = "Check config.toml for errors, unknown keys, and bad values.",
        long_about = "Parses config.toml and reports unknown keys, invalid values, and settings \
that chat would reject or ignore at startup. Exits 1 when any error is found. A missing file at \
the default path is fine; defaults apply. With --store, also checks <DIR>/config.toml on its own \
and then the merged result."
    )]
    Check {
        #[arg(long, help = "Config file to check instead of the default path")]
        path: Option<PathBuf>,
        #[arg(
            long,
            help = "Chat store directory; also check its config.toml and the merged result"
        )]
        store: Option<PathBuf>,
    },
    #[command(
        about = "Show the merged config with the source of each value.",
        long_about = "Prints every setting of the merged config with its source: default, global \
(config.toml in the config directory) or store (<DIR>/config.toml). A setting whose environment \
variable is set is marked. Command-line flags do not apply to this command, so they are not shown."
    )]
    Show {
        #[arg(
            long,
            help = "Chat store directory whose config.toml overrides the global file"
        )]
        store: Option<PathBuf>,
    },
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireProvider {
    Anthropic,
}

#[derive(clap::Args, Debug, Clone)]
#[command(args_conflicts_with_subcommands = true)]
pub struct WireArgs {
    #[command(subcommand)]
    pub command: Option<WireCommand>,

    #[arg(
        long,
        value_enum,
        default_value = "anthropic",
        help = "Provider to proxy"
    )]
    pub provider: WireProvider,

    #[arg(
        long,
        default_value_t = 0,
        help = "Port on 127.0.0.1 (0 picks a free port)"
    )]
    pub port: u16,

    #[arg(
        long,
        value_name = "URL",
        help = "Upstream base URL (default: the provider's)"
    )]
    pub upstream: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Directory for session folders (default: <tmp>/ralph-wire)"
    )]
    pub dir: Option<PathBuf>,

    #[arg(long, help = "Also store the decoded SSE events of each response")]
    pub responses: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum WireCommand {
    #[command(about = "Show one row per inference call of a session, plus totals.")]
    Ls(WireLsArgs),

    #[command(about = "Show the prompt segments, parameters, and usage of one recorded call.")]
    Show(WireShowArgs),

    #[command(
        about = "Explain why a call missed the prompt cache by diffing it against an earlier one."
    )]
    Diff(WireDiffArgs),

    #[command(about = "List recorded sessions.")]
    Sessions(WireSessionsArgs),
}

#[derive(clap::Args, Debug, Clone)]
pub struct WireLsArgs {
    #[arg(
        long,
        value_name = "ID",
        help = "Session to show (default: the latest)"
    )]
    pub session: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Directory holding session folders (default: <tmp>/ralph-wire)"
    )]
    pub dir: Option<PathBuf>,

    #[arg(long, help = "Also show non-inference calls")]
    pub all: bool,

    #[arg(long, help = "Print JSON instead of a table")]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct WireShowArgs {
    #[arg(value_name = "N", help = "Call number (seq) as printed by `wire ls`")]
    pub n: u64,

    #[arg(
        long,
        value_name = "ID",
        help = "Session to show (default: the latest)"
    )]
    pub session: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Directory holding session folders (default: <tmp>/ralph-wire)"
    )]
    pub dir: Option<PathBuf>,

    #[arg(long, help = "Print JSON instead of a table")]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct WireDiffArgs {
    #[arg(
        value_name = "A",
        help = "Earlier call (seq); with B omitted, the call to explain"
    )]
    pub a: u64,

    #[arg(value_name = "B", help = "Later call (seq) to compare against A")]
    pub b: Option<u64>,

    #[arg(
        long,
        value_name = "ID",
        help = "Session to show (default: the latest)"
    )]
    pub session: Option<String>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Directory holding session folders (default: <tmp>/ralph-wire)"
    )]
    pub dir: Option<PathBuf>,

    #[arg(long, help = "Print JSON instead of a table")]
    pub json: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct WireSessionsArgs {
    #[arg(
        long,
        value_name = "PATH",
        help = "Directory holding session folders (default: <tmp>/ralph-wire)"
    )]
    pub dir: Option<PathBuf>,

    #[arg(long, help = "Print JSON instead of a table")]
    pub json: bool,
}

#[derive(clap::Args, Debug)]
pub struct MermaidArgs {
    #[arg(long)]
    pub width: usize,
}

#[derive(clap::Args, Debug)]
pub struct ChatArgs {
    #[arg(long, value_name = "DIR")]
    pub store: Option<std::path::PathBuf>,

    #[arg(long, value_name = "TOKENS")]
    pub recent_history_tokens: Option<usize>,

    #[arg(long, value_name = "COLUMNS")]
    pub wrap_width: Option<usize>,

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

    #[arg(long, value_name = "PATH")]
    pub socket: std::path::PathBuf,
}

#[derive(clap::Args, Debug)]
pub struct WorkerMcpArgs {
    #[arg(long, value_name = "WORKER")]
    pub id: String,

    #[arg(long, value_name = "DIR")]
    pub inbox: std::path::PathBuf,

    #[arg(long, value_name = "PATH")]
    pub owner: Option<std::path::PathBuf>,

    #[arg(long, value_name = "DIR")]
    pub worktree: Option<std::path::PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct WorkerStopHookArgs {
    #[arg(long, value_name = "PATH")]
    pub reported: std::path::PathBuf,

    #[arg(long, value_name = "PATH")]
    pub unreported: std::path::PathBuf,

    #[arg(long, value_name = "WORKER", requires = "usage_seen")]
    pub worker: Option<String>,

    #[arg(long, value_name = "PATH", requires = "worker")]
    pub usage_seen: Option<std::path::PathBuf>,

    #[arg(
        long,
        value_name = "PATH",
        requires = "usage_seen",
        help = "Chat usage.jsonl the hook appends worker rows to"
    )]
    pub usage: Option<std::path::PathBuf>,

    #[arg(long, value_name = "DIR", requires_all = ["id", "worktree"])]
    pub store: Option<std::path::PathBuf>,

    #[arg(long, value_name = "WORKER")]
    pub id: Option<String>,

    #[arg(long, value_name = "DIR")]
    pub worktree: Option<std::path::PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct ChatNapArgs {
    #[arg(long, value_name = "DIR")]
    pub store: std::path::PathBuf,

    #[arg(long, value_name = "ID")]
    pub run: Option<String>,

    #[arg(long, default_value_t = NAP_TIMEOUT_SECS)]
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
