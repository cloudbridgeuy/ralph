//! Tests for the `persona` subcommand CLI argument parsing.

use crate::cli::{Cli, Commands, PersonaAction, PersonaArgs};
use clap::Parser;

/// Helper function to parse CLI args and extract PersonaArgs.
fn parse_persona_args(args: &[&str]) -> PersonaArgs {
    let cli = Cli::try_parse_from(args).expect("Failed to parse CLI args");
    match cli.command {
        Commands::Persona(args) => args,
        _ => panic!("Expected Persona command"),
    }
}

#[test]
fn test_persona_with_name_parses() {
    let args = parse_persona_args(&["ralph", "persona", "dev"]);
    assert_eq!(args.persona, Some("dev".to_string()));
    assert!(!args.list);
}

#[test]
fn test_persona_list_flag_parses() {
    let args = parse_persona_args(&["ralph", "persona", "--list"]);
    assert!(args.list);
    assert!(args.persona.is_none());
}

#[test]
fn test_persona_list_conflicts_with_name() {
    let result = Cli::try_parse_from(["ralph", "persona", "dev", "--list"]);
    assert!(result.is_err());
}

#[test]
fn test_into_action_list() {
    let args = parse_persona_args(&["ralph", "persona", "--list"]);
    let action = args.into_action().unwrap();
    assert!(matches!(action, PersonaAction::List));
}

#[test]
fn test_into_action_invoke() {
    let args = parse_persona_args(&[
        "ralph",
        "persona",
        "dev",
        "--session",
        "my-session",
        "--continue",
        "--theme",
        "Monokai Extended",
        "--no-background",
        "--timeout",
        "120",
        "--no-prompt",
        "--history",
        "--clone-session",
        "hello",
    ]);
    let action = args.into_action().unwrap();
    match action {
        PersonaAction::Invoke(invoke) => {
            assert_eq!(invoke.persona, "dev");
            assert_eq!(invoke.prompt, Some("hello".to_string()));
            assert_eq!(invoke.session, Some("my-session".to_string()));
            assert!(invoke.continue_session);
            assert_eq!(invoke.theme, Some("Monokai Extended".to_string()));
            assert!(invoke.no_background);
            assert_eq!(invoke.timeout, 120);
            assert!(invoke.no_prompt);
            assert!(invoke.history);
            assert!(invoke.clone_session);
        }
        PersonaAction::List => panic!("Expected Invoke action"),
    }
}

#[test]
fn test_into_action_no_name_no_list() {
    let args = parse_persona_args(&["ralph", "persona"]);
    let result = args.into_action();
    assert!(result.is_err());
}
