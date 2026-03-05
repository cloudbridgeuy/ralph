//! Tests for the `strategy` subcommand CLI argument parsing.

use crate::cli::{Cli, Commands, StrategyAction};
use clap::Parser;

#[test]
fn test_strategy_list_parses() {
    let cli = Cli::try_parse_from(["ralph", "strategy", "list"]).unwrap();
    match cli.command {
        Commands::Strategy(args) => {
            assert!(matches!(args.action, StrategyAction::List));
        }
        _ => panic!("Expected Strategy command"),
    }
}

#[test]
fn test_strategy_execute_parses_name() {
    let cli = Cli::try_parse_from(["ralph", "strategy", "execute", "prd-loop"]).unwrap();
    match cli.command {
        Commands::Strategy(args) => match args.action {
            StrategyAction::Execute(exec_args) => {
                assert_eq!(exec_args.name, "prd-loop");
                assert!(exec_args.max_iterations.is_none());
                assert!(!exec_args.resume);
            }
            _ => panic!("Expected Execute action"),
        },
        _ => panic!("Expected Strategy command"),
    }
}

#[test]
fn test_strategy_execute_with_max_iterations() {
    let cli = Cli::try_parse_from([
        "ralph",
        "strategy",
        "execute",
        "prd-loop",
        "--max-iterations",
        "5",
    ])
    .unwrap();
    match cli.command {
        Commands::Strategy(args) => match args.action {
            StrategyAction::Execute(exec_args) => {
                assert_eq!(exec_args.name, "prd-loop");
                assert_eq!(exec_args.max_iterations, Some(5));
            }
            _ => panic!("Expected Execute action"),
        },
        _ => panic!("Expected Strategy command"),
    }
}

#[test]
fn test_strategy_execute_with_resume() {
    let cli =
        Cli::try_parse_from(["ralph", "strategy", "execute", "prd-loop", "--resume"]).unwrap();
    match cli.command {
        Commands::Strategy(args) => match args.action {
            StrategyAction::Execute(exec_args) => {
                assert_eq!(exec_args.name, "prd-loop");
                assert!(exec_args.resume);
            }
            _ => panic!("Expected Execute action"),
        },
        _ => panic!("Expected Strategy command"),
    }
}

#[test]
fn test_strategy_execute_with_all_flags() {
    let cli = Cli::try_parse_from([
        "ralph",
        "strategy",
        "execute",
        "prd-loop",
        "--max-iterations",
        "10",
        "--resume",
    ])
    .unwrap();
    match cli.command {
        Commands::Strategy(args) => match args.action {
            StrategyAction::Execute(exec_args) => {
                assert_eq!(exec_args.name, "prd-loop");
                assert_eq!(exec_args.max_iterations, Some(10));
                assert!(exec_args.resume);
            }
            _ => panic!("Expected Execute action"),
        },
        _ => panic!("Expected Strategy command"),
    }
}

#[test]
fn test_strategy_execute_requires_name() {
    let result = Cli::try_parse_from(["ralph", "strategy", "execute"]);
    assert!(result.is_err());
}

#[test]
fn test_strategy_without_subcommand_fails() {
    let result = Cli::try_parse_from(["ralph", "strategy"]);
    assert!(result.is_err());
}
