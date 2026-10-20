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
fn test_cli_version_flags_render_the_current_version() {
    for flag in ["--version", "-V"] {
        let err = Cli::try_parse_from(["ralph", flag]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(crate::version::current()));
    }
}

#[test]
fn test_cli_chat_takes_recent_history_tokens() {
    let Some(Commands::Chat(chat)) =
        Cli::try_parse_from(["ralph", "chat", "--recent-history-tokens", "0"])
            .unwrap()
            .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.recent_history_tokens, Some(0));
}

#[test]
fn test_cli_chat_takes_a_wrap_width() {
    let Some(Commands::Chat(chat)) = Cli::try_parse_from(["ralph", "chat", "--wrap-width", "80"])
        .unwrap()
        .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.wrap_width, Some(80));
}

#[test]
fn test_cli_chat_takes_a_worktree_dir_and_chat_mcp_a_socket() {
    let Some(Commands::Chat(chat)) =
        Cli::try_parse_from(["ralph", "chat", "--worktree-dir", "~/wt"])
            .unwrap()
            .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.worktree_dir.as_deref(), Some("~/wt"));
    let Some(Commands::ChatMcp(mcp)) = Cli::try_parse_from([
        "ralph",
        "chat-mcp",
        "--store",
        "/s",
        "--socket",
        "/s/chat-1.sock",
    ])
    .unwrap()
    .command
    else {
        panic!("expected chat-mcp");
    };
    assert_eq!(mcp.socket, std::path::PathBuf::from("/s/chat-1.sock"));
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
