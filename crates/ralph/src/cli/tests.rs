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
fn test_cli_chat_and_chat_mcp_take_a_worktree_dir() {
    let Some(Commands::Chat(chat)) =
        Cli::try_parse_from(["ralph", "chat", "--worktree-dir", "~/wt"])
            .unwrap()
            .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.worktree_dir.as_deref(), Some("~/wt"));
    let Some(Commands::ChatMcp(mcp)) =
        Cli::try_parse_from(["ralph", "chat-mcp", "--store", "/s", "--worktree-dir", "wt"])
            .unwrap()
            .command
    else {
        panic!("expected chat-mcp");
    };
    assert_eq!(mcp.worktree_dir.as_deref(), Some("wt"));
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
