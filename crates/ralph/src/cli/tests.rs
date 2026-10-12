//! Tests for CLI argument parsing.

#[path = "tests/ask.rs"]
mod ask;

use super::*;

#[test]
fn test_cli_bare_ralph_has_no_subcommand() {
    let cli = Cli::try_parse_from(["ralph"]).unwrap();
    assert!(cli.command.is_none());
}

#[test]
fn test_cli_rejects_removed_session_commands() {
    for args in [
        ["ralph", "replay", "quiet-mountain"],
        ["ralph", "sessions", "--all"],
    ] {
        assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
    }
}
