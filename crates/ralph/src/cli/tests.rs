//! Tests for CLI argument parsing.

#[path = "tests/ask.rs"]
mod ask;

use super::*;
use std::path::PathBuf;

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
fn test_run_with_max_attempts() {
    let cli = Cli::try_parse_from(["ralph", "run", "--max-attempts", "5"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.max_attempts, 5);
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_default_max_attempts() {
    let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.max_attempts, 3);
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
        "--max-attempts",
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
            assert_eq!(args.max_attempts, 2);
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

#[test]
fn test_run_with_additional_prompt() {
    let cli = Cli::try_parse_from([
        "ralph",
        "run",
        "--additional-prompt",
        "Extra instructions here",
    ])
    .unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(
                args.additional_prompt,
                Some("Extra instructions here".to_string())
            );
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_additional_prompt_short_flag() {
    let cli = Cli::try_parse_from(["ralph", "run", "-a", "Extra instructions"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(
                args.additional_prompt,
                Some("Extra instructions".to_string())
            );
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_without_additional_prompt() {
    let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert!(args.additional_prompt.is_none());
        }
        _ => panic!("Expected Run command"),
    }
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

#[test]
fn test_sessions_with_interrupted_outcome() {
    let cli = Cli::try_parse_from(["ralph", "sessions", "--outcome", "interrupted"]).unwrap();
    match cli.command {
        Commands::Sessions(args) => {
            assert_eq!(args.outcome, Some("interrupted".to_string()));
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
    let cli = Cli::try_parse_from(["ralph", "replay", "my-session", "--iteration", "5"]).unwrap();
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

#[test]
fn test_replay_with_delay() {
    let cli = Cli::try_parse_from(["ralph", "replay", "my-session", "--delay", "2.5"]).unwrap();
    match cli.command {
        Commands::Replay(args) => {
            assert_eq!(args.slug, "my-session");
            assert_eq!(args.delay, Some(2.5));
        }
        _ => panic!("Expected Replay command"),
    }
}

#[test]
fn test_replay_with_delay_integer() {
    let cli = Cli::try_parse_from(["ralph", "replay", "my-session", "--delay", "2"]).unwrap();
    match cli.command {
        Commands::Replay(args) => {
            assert_eq!(args.delay, Some(2.0));
        }
        _ => panic!("Expected Replay command"),
    }
}

#[test]
fn test_replay_with_delay_fractional() {
    let cli = Cli::try_parse_from(["ralph", "replay", "my-session", "--delay", "0.5"]).unwrap();
    match cli.command {
        Commands::Replay(args) => {
            assert_eq!(args.delay, Some(0.5));
        }
        _ => panic!("Expected Replay command"),
    }
}

#[test]
fn test_replay_without_delay() {
    let cli = Cli::try_parse_from(["ralph", "replay", "my-session"]).unwrap();
    match cli.command {
        Commands::Replay(args) => {
            assert!(args.delay.is_none());
        }
        _ => panic!("Expected Replay command"),
    }
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
    let cli = Cli::try_parse_from(["ralph", "iterations", "--session", "quiet-mountain"]).unwrap();
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

// Progress summarization flag tests

#[test]
fn test_run_default_progress_max_lines() {
    let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.progress_max_lines, 1000);
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_progress_max_lines() {
    let cli = Cli::try_parse_from(["ralph", "run", "--progress-max-lines", "500"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.progress_max_lines, 500);
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_summarize_command() {
    let cli =
        Cli::try_parse_from(["ralph", "run", "--summarize-command", "my-llm -p {prompt}"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(
                args.summarize_command,
                Some("my-llm -p {prompt}".to_string())
            );
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_summarize_prompt() {
    let cli = Cli::try_parse_from([
        "ralph",
        "run",
        "--summarize-prompt",
        "Summarize: {progress_content}",
    ])
    .unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(
                args.summarize_prompt,
                Some("Summarize: {progress_content}".to_string())
            );
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_no_summarize() {
    let cli = Cli::try_parse_from(["ralph", "run", "--no-summarize"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert!(args.no_summarize);
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_default_no_summarize() {
    let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert!(!args.no_summarize);
        }
        _ => panic!("Expected Run command"),
    }
}

// Verbose tools flag tests

#[test]
fn test_run_without_verbose_tools() {
    let cli = Cli::try_parse_from(["ralph", "run"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert!(args.verbose_tools.is_none());
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_verbose_tools_all() {
    // --verbose-tools without value enables all tools (uses default_missing_value)
    let cli = Cli::try_parse_from(["ralph", "run", "--verbose-tools"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.verbose_tools, Some("*".to_string()));
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_verbose_tools_specific() {
    let cli = Cli::try_parse_from(["ralph", "run", "--verbose-tools=grep,bash"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.verbose_tools, Some("grep,bash".to_string()));
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_verbose_tools_single() {
    let cli = Cli::try_parse_from(["ralph", "run", "--verbose-tools=read"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.verbose_tools, Some("read".to_string()));
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_run_with_verbose_tools_space_separated() {
    let cli = Cli::try_parse_from(["ralph", "run", "--verbose-tools", "grep,read"]).unwrap();
    match cli.command {
        Commands::Run(args) => {
            assert_eq!(args.verbose_tools, Some("grep,read".to_string()));
        }
        _ => panic!("Expected Run command"),
    }
}
