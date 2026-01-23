//! Tests for the `ask` subcommand CLI argument parsing.

use crate::cli::{AskArgs, Cli, Commands};
use clap::Parser;

/// Helper function to parse CLI args and extract AskArgs.
///
/// Reduces boilerplate in tests by handling the common pattern of
/// parsing arguments and matching the Ask command variant.
fn parse_ask_args(args: &[&str]) -> AskArgs {
    let cli = Cli::try_parse_from(args).expect("Failed to parse CLI args");
    match cli.command {
        Commands::Ask(args) => args,
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_cli_parses_ask_command() {
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    assert!(matches!(cli.command, Commands::Ask(_)));
}

#[test]
fn test_ask_with_prompt() {
    let args = parse_ask_args(&["ralph", "ask", "what is 2+2"]);
    assert_eq!(args.prompt, Some("what is 2+2".to_string()));
}

#[test]
fn test_ask_without_prompt() {
    let args = parse_ask_args(&["ralph", "ask"]);
    assert!(args.prompt.is_none());
}

#[test]
fn test_ask_help_available() {
    // Verify help is available
    let result = Cli::try_parse_from(["ralph", "ask", "--help"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let help = err.to_string();
    assert!(help.contains("PROMPT"));
}

#[test]
fn test_ask_with_stdin_indicator() {
    let cli = Cli::try_parse_from(["ralph", "ask", "-"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.prompt, Some("-".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_multiword_prompt() {
    let cli = Cli::try_parse_from(["ralph", "ask", "explain the difference between mut and ref"])
        .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(
                args.prompt,
                Some("explain the difference between mut and ref".to_string())
            );
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_session() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--session", "my-session", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.session, Some("my-session".to_string()));
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_session_short_flag() {
    let cli = Cli::try_parse_from(["ralph", "ask", "-S", "my-test", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.session, Some("my-test".to_string()));
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_without_session() {
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.session.is_none());
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_session_before_prompt() {
    // Session flag can come before the positional prompt
    let cli =
        Cli::try_parse_from(["ralph", "ask", "--session", "test-session", "my prompt"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.session, Some("test-session".to_string()));
            assert_eq!(args.prompt, Some("my prompt".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_session_after_prompt() {
    // Session flag can come after the positional prompt
    let cli =
        Cli::try_parse_from(["ralph", "ask", "my prompt", "--session", "test-session"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.session, Some("test-session".to_string()));
            assert_eq!(args.prompt, Some("my prompt".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_continue_flag() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--continue", "follow up"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.continue_session);
            assert_eq!(args.prompt, Some("follow up".to_string()));
            assert!(args.session.is_none());
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_continue_short_flag() {
    let cli = Cli::try_parse_from(["ralph", "ask", "-c", "follow up"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.continue_session);
            assert_eq!(args.prompt, Some("follow up".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_continue_and_session() {
    // Continue a specific named session
    let cli = Cli::try_parse_from([
        "ralph",
        "ask",
        "--session",
        "my-test",
        "--continue",
        "follow up",
    ])
    .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.continue_session);
            assert_eq!(args.session, Some("my-test".to_string()));
            assert_eq!(args.prompt, Some("follow up".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_continue_session_short_flags() {
    let cli = Cli::try_parse_from(["ralph", "ask", "-S", "my-test", "-c", "follow up"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.continue_session);
            assert_eq!(args.session, Some("my-test".to_string()));
            assert_eq!(args.prompt, Some("follow up".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_without_continue_flag() {
    // Default: continue_session is false
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(!args.continue_session);
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

// Theme flag tests for ask command

#[test]
fn test_ask_with_theme() {
    let cli =
        Cli::try_parse_from(["ralph", "ask", "--theme", "Monokai Extended", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.theme, Some("Monokai Extended".to_string()));
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_no_background() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--no-background", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.no_background);
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_without_theme_flags() {
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.theme.is_none());
            assert!(!args.no_background);
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_theme_and_no_background() {
    let cli = Cli::try_parse_from([
        "ralph",
        "ask",
        "--theme",
        "Solarized (dark)",
        "--no-background",
        "hello",
    ])
    .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.theme, Some("Solarized (dark)".to_string()));
            assert!(args.no_background);
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_theme_with_file_path() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--theme", "/path/to/theme.tmTheme", "hello"])
        .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.theme, Some("/path/to/theme.tmTheme".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_theme_and_continue() {
    let cli = Cli::try_parse_from([
        "ralph",
        "ask",
        "--theme",
        "Monokai Extended",
        "--no-background",
        "--continue",
        "--session",
        "my-test",
        "follow up",
    ])
    .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.theme, Some("Monokai Extended".to_string()));
            assert!(args.no_background);
            assert!(args.continue_session);
            assert_eq!(args.session, Some("my-test".to_string()));
            assert_eq!(args.prompt, Some("follow up".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

// Timeout, verbose-tools, and no-prompt flag tests for ask command

#[test]
fn test_ask_with_timeout() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--timeout", "60", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.timeout, 60);
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_timeout_default() {
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.timeout, 600); // Default 10 minutes
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_verbose_tools_all() {
    // --verbose-tools without value enables all tools (uses default_missing_value)
    // Place prompt before the flag so clap knows the flag has no value
    let cli = Cli::try_parse_from(["ralph", "ask", "hello", "--verbose-tools"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.verbose_tools, Some("*".to_string()));
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_verbose_tools_specific() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--verbose-tools=grep,bash", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.verbose_tools, Some("grep,bash".to_string()));
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_without_verbose_tools() {
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.verbose_tools.is_none());
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_no_prompt_flag() {
    let cli = Cli::try_parse_from(["ralph", "ask", "--no-prompt", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(args.no_prompt);
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_without_no_prompt_flag() {
    let cli = Cli::try_parse_from(["ralph", "ask", "hello"]).unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert!(!args.no_prompt);
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_all_new_flags() {
    let cli = Cli::try_parse_from([
        "ralph",
        "ask",
        "--timeout",
        "120",
        "--verbose-tools=read",
        "--no-prompt",
        "hello",
    ])
    .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.timeout, 120);
            assert_eq!(args.verbose_tools, Some("read".to_string()));
            assert!(args.no_prompt);
            assert_eq!(args.prompt, Some("hello".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}

#[test]
fn test_ask_with_all_flags_combined() {
    // Test combining all ask flags together
    let cli = Cli::try_parse_from([
        "ralph",
        "ask",
        "--session",
        "my-test",
        "--continue",
        "--theme",
        "Monokai Extended",
        "--no-background",
        "--timeout",
        "300",
        "--verbose-tools",
        "--no-prompt",
        "follow up",
    ])
    .unwrap();
    match cli.command {
        Commands::Ask(args) => {
            assert_eq!(args.session, Some("my-test".to_string()));
            assert!(args.continue_session);
            assert_eq!(args.theme, Some("Monokai Extended".to_string()));
            assert!(args.no_background);
            assert_eq!(args.timeout, 300);
            assert_eq!(args.verbose_tools, Some("*".to_string()));
            assert!(args.no_prompt);
            assert_eq!(args.prompt, Some("follow up".to_string()));
        }
        _ => panic!("Expected Ask command"),
    }
}
