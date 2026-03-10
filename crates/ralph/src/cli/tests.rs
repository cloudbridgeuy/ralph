//! Tests for CLI argument parsing.

#[path = "tests/ask.rs"]
mod ask;

#[path = "tests/persona.rs"]
mod persona;

#[path = "tests/strategy.rs"]
mod strategy;

use super::*;

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
