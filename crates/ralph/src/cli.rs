//! CLI argument parsing for ralph.
//!
//! This module defines the command-line interface using clap derive macros.
//! It provides the main CLI struct and all subcommand definitions.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
}

/// Arguments for the `sessions` subcommand.
#[derive(clap::Args, Debug)]
pub struct SessionsArgs {
    /// Filter sessions by project path (substring match).
    #[arg(long)]
    pub project: Option<String>,

    /// Filter sessions by outcome status.
    ///
    /// Valid values: in_progress, completed, aborted, failed
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
    /// Use a built-in theme name or a path to a custom .tmTheme file.
    /// Can also be set via RALPH_THEME environment variable.
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// Disable background colors in syntax highlighting.
    ///
    /// Can also be set via RALPH_NO_BACKGROUND environment variable.
    #[arg(long)]
    pub no_background: bool,
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
    /// Placeholders: {design_file}, {prd_file}, {progress_file}
    #[arg(short, long)]
    pub prompt: Option<String>,

    /// Custom LLM invocation pattern.
    ///
    /// Command template to execute with {prompt} placeholder.
    /// Default: claude --permission-mode acceptEdits --output-format stream-json -p {prompt}
    #[arg(short, long, value_name = "TEMPLATE")]
    pub command: Option<String>,

    /// Design document path.
    ///
    /// Default: .local/designs/design.md
    #[arg(long, value_name = "PATH")]
    pub design: Option<PathBuf>,

    /// PRD file path.
    ///
    /// Default: .local/plans/prd.toml
    #[arg(long, value_name = "PATH")]
    pub prd: Option<PathBuf>,

    /// Progress notes path.
    ///
    /// Default: .local/plans/progress.txt
    #[arg(long, value_name = "PATH")]
    pub progress: Option<PathBuf>,

    /// Auto-retry count on failure.
    ///
    /// Number of times to automatically retry if the LLM subprocess fails.
    /// After exhausting retries, prompts user for action.
    #[arg(long, default_value_t = 3)]
    pub retry: usize,

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parses_run_command() {
        let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn test_run_with_iterations() {
        let cli = Cli::try_parse_from(["ralph", "run", "5"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.iterations, Some(5));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_slug() {
        let cli = Cli::try_parse_from(["ralph", "run", "--slug", "my-session"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.slug, Some("my-session".to_string()));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_short_slug() {
        let cli = Cli::try_parse_from(["ralph", "run", "-s", "my-session"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.slug, Some("my-session".to_string()));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_prompt() {
        let cli = Cli::try_parse_from(["ralph", "run", "--prompt", "custom prompt"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.prompt, Some("custom prompt".to_string()));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_command_template() {
        let cli = Cli::try_parse_from(["ralph", "run", "--command", "echo {prompt}"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.command, Some("echo {prompt}".to_string()));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_path_overrides() {
        let cli = Cli::try_parse_from([
            "ralph",
            "run",
            "--design",
            "/custom/design.md",
            "--prd",
            "/custom/prd.toml",
            "--progress",
            "/custom/progress.txt",
        ])
        .unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.design, Some(PathBuf::from("/custom/design.md")));
                assert_eq!(args.prd, Some(PathBuf::from("/custom/prd.toml")));
                assert_eq!(args.progress, Some(PathBuf::from("/custom/progress.txt")));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_retry() {
        let cli = Cli::try_parse_from(["ralph", "run", "--retry", "5"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.retry, 5);
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_default_retry() {
        let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.retry, 3);
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_completion_marker() {
        let cli = Cli::try_parse_from(["ralph", "run", "--completion-marker", "DONE"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.completion_marker, Some("DONE".to_string()));
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_timeout() {
        let cli = Cli::try_parse_from(["ralph", "run", "--timeout", "300"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.timeout, 300);
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_default_timeout() {
        let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.timeout, 600); // Default is 10 minutes
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_with_all_options() {
        let cli = Cli::try_parse_from([
            "ralph",
            "run",
            "10",
            "--slug",
            "test-run",
            "--prompt",
            "test prompt",
            "--command",
            "echo {prompt}",
            "--design",
            "/d.md",
            "--prd",
            "/p.toml",
            "--progress",
            "/pr.txt",
            "--retry",
            "2",
            "--completion-marker",
            "END",
            "--timeout",
            "120",
        ])
        .unwrap();
        match cli.command {
            Commands::Run(args) => {
                assert_eq!(args.iterations, Some(10));
                assert_eq!(args.slug, Some("test-run".to_string()));
                assert_eq!(args.prompt, Some("test prompt".to_string()));
                assert_eq!(args.command, Some("echo {prompt}".to_string()));
                assert_eq!(args.design, Some(PathBuf::from("/d.md")));
                assert_eq!(args.prd, Some(PathBuf::from("/p.toml")));
                assert_eq!(args.progress, Some(PathBuf::from("/pr.txt")));
                assert_eq!(args.retry, 2);
                assert_eq!(args.completion_marker, Some("END".to_string()));
                assert_eq!(args.timeout, 120);
            }
            _ => panic!("Expected Run command"),
        }
    }

    #[test]
    fn test_run_help_includes_default_command() {
        // Verify help text mentions the default command
        let result = Cli::try_parse_from(["ralph", "run", "--help"]);
        // --help causes an error with the help message
        assert!(result.is_err());
        let err = result.unwrap_err();
        let help = err.to_string();
        assert!(help.contains("ITERATIONS"));
    }

    // Sessions command tests

    #[test]
    fn test_cli_parses_sessions_command() {
        let cli = Cli::try_parse_from(["ralph", "sessions"]).unwrap();
        assert!(matches!(cli.command, Commands::Sessions(_)));
    }

    #[test]
    fn test_sessions_without_args() {
        let cli = Cli::try_parse_from(["ralph", "sessions"]).unwrap();
        match cli.command {
            Commands::Sessions(args) => {
                assert!(args.project.is_none());
                assert!(args.outcome.is_none());
            }
            _ => panic!("Expected Sessions command"),
        }
    }

    #[test]
    fn test_sessions_with_project_filter() {
        let cli = Cli::try_parse_from(["ralph", "sessions", "--project", "/my/project"]).unwrap();
        match cli.command {
            Commands::Sessions(args) => {
                assert_eq!(args.project, Some("/my/project".to_string()));
            }
            _ => panic!("Expected Sessions command"),
        }
    }

    #[test]
    fn test_sessions_with_outcome_filter() {
        let cli = Cli::try_parse_from(["ralph", "sessions", "--outcome", "completed"]).unwrap();
        match cli.command {
            Commands::Sessions(args) => {
                assert_eq!(args.outcome, Some("completed".to_string()));
            }
            _ => panic!("Expected Sessions command"),
        }
    }

    #[test]
    fn test_sessions_with_both_filters() {
        let cli = Cli::try_parse_from([
            "ralph",
            "sessions",
            "--project",
            "myproject",
            "--outcome",
            "in_progress",
        ])
        .unwrap();
        match cli.command {
            Commands::Sessions(args) => {
                assert_eq!(args.project, Some("myproject".to_string()));
                assert_eq!(args.outcome, Some("in_progress".to_string()));
            }
            _ => panic!("Expected Sessions command"),
        }
    }

    // Replay command tests

    #[test]
    fn test_cli_parses_replay_command() {
        let cli = Cli::try_parse_from(["ralph", "replay", "my-session"]).unwrap();
        assert!(matches!(cli.command, Commands::Replay(_)));
    }

    #[test]
    fn test_replay_with_slug() {
        let cli = Cli::try_parse_from(["ralph", "replay", "quiet-mountain"]).unwrap();
        match cli.command {
            Commands::Replay(args) => {
                assert_eq!(args.slug, "quiet-mountain");
                assert!(args.iteration.is_none());
            }
            _ => panic!("Expected Replay command"),
        }
    }

    #[test]
    fn test_replay_with_iteration() {
        let cli = Cli::try_parse_from(["ralph", "replay", "my-session", "-i", "3"]).unwrap();
        match cli.command {
            Commands::Replay(args) => {
                assert_eq!(args.slug, "my-session");
                assert_eq!(args.iteration, Some(3));
            }
            _ => panic!("Expected Replay command"),
        }
    }

    #[test]
    fn test_replay_with_iteration_long_flag() {
        let cli =
            Cli::try_parse_from(["ralph", "replay", "my-session", "--iteration", "5"]).unwrap();
        match cli.command {
            Commands::Replay(args) => {
                assert_eq!(args.slug, "my-session");
                assert_eq!(args.iteration, Some(5));
            }
            _ => panic!("Expected Replay command"),
        }
    }

    #[test]
    fn test_replay_requires_slug() {
        // Replay without slug should fail
        let result = Cli::try_parse_from(["ralph", "replay"]);
        assert!(result.is_err());
    }

    // Iterations command tests

    #[test]
    fn test_cli_parses_iterations_command() {
        let cli = Cli::try_parse_from(["ralph", "iterations"]).unwrap();
        assert!(matches!(cli.command, Commands::Iterations(_)));
    }

    #[test]
    fn test_iterations_without_args() {
        let cli = Cli::try_parse_from(["ralph", "iterations"]).unwrap();
        match cli.command {
            Commands::Iterations(args) => {
                assert!(args.session.is_none());
                assert!(args.project.is_none());
                assert!(args.outcome.is_none());
            }
            _ => panic!("Expected Iterations command"),
        }
    }

    #[test]
    fn test_iterations_with_session_filter() {
        let cli =
            Cli::try_parse_from(["ralph", "iterations", "--session", "quiet-mountain"]).unwrap();
        match cli.command {
            Commands::Iterations(args) => {
                assert_eq!(args.session, Some("quiet-mountain".to_string()));
            }
            _ => panic!("Expected Iterations command"),
        }
    }

    #[test]
    fn test_iterations_with_project_filter() {
        let cli = Cli::try_parse_from(["ralph", "iterations", "--project", "/my/project"]).unwrap();
        match cli.command {
            Commands::Iterations(args) => {
                assert_eq!(args.project, Some("/my/project".to_string()));
            }
            _ => panic!("Expected Iterations command"),
        }
    }

    #[test]
    fn test_iterations_with_outcome_filter() {
        let cli = Cli::try_parse_from(["ralph", "iterations", "--outcome", "completed"]).unwrap();
        match cli.command {
            Commands::Iterations(args) => {
                assert_eq!(args.outcome, Some("completed".to_string()));
            }
            _ => panic!("Expected Iterations command"),
        }
    }

    #[test]
    fn test_iterations_with_all_filters() {
        let cli = Cli::try_parse_from([
            "ralph",
            "iterations",
            "--session",
            "my-session",
            "--project",
            "myproject",
            "--outcome",
            "failed",
        ])
        .unwrap();
        match cli.command {
            Commands::Iterations(args) => {
                assert_eq!(args.session, Some("my-session".to_string()));
                assert_eq!(args.project, Some("myproject".to_string()));
                assert_eq!(args.outcome, Some("failed".to_string()));
            }
            _ => panic!("Expected Iterations command"),
        }
    }
}
