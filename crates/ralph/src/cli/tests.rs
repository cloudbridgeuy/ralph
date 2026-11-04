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
fn test_cli_chat_nap_takes_store_and_run() {
    let Some(Commands::ChatNap(nap)) =
        Cli::try_parse_from(["ralph", "chat-nap", "--store", "X", "--run", "R"])
            .unwrap()
            .command
    else {
        panic!("expected chat-nap");
    };
    assert_eq!(nap.store, std::path::PathBuf::from("X"));
    assert_eq!(nap.run.as_deref(), Some("R"));
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
fn test_cli_chat_takes_a_name() {
    let Some(Commands::Chat(chat)) = Cli::try_parse_from(["ralph", "chat", "--name", "review"])
        .unwrap()
        .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.name.as_deref(), Some("review"));
    let Some(Commands::Chat(chat)) = Cli::try_parse_from(["ralph", "chat"]).unwrap().command else {
        panic!("expected chat");
    };
    assert_eq!(chat.name, None);
}

#[test]
fn test_cli_chat_history_rejects_a_name() {
    let err = Cli::try_parse_from(["ralph", "chat", "history", "zoom", "4-7", "--name", "x"])
        .unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::UnknownArgument);
}

#[test]
fn test_cli_chat_name_flag_still_parses_for_a_bare_chat() {
    let Some(Commands::Chat(chat)) = Cli::try_parse_from(["ralph", "chat", "--name", "x"])
        .unwrap()
        .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.name.as_deref(), Some("x"));
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

#[test]
fn test_cli_wire_defaults_and_flags() {
    let Some(Commands::Wire(wire)) = Cli::try_parse_from(["ralph", "wire"]).unwrap().command else {
        panic!("expected wire");
    };
    assert_eq!(wire.provider, WireProvider::Anthropic);
    assert_eq!(wire.port, 0);
    assert!(wire.upstream.is_none() && wire.dir.is_none() && !wire.responses);

    let Some(Commands::Wire(wire)) = Cli::try_parse_from([
        "ralph",
        "wire",
        "--port",
        "9",
        "--upstream",
        "http://u",
        "--dir",
        "/d",
        "--responses",
    ])
    .unwrap()
    .command
    else {
        panic!("expected wire");
    };
    assert_eq!(wire.port, 9);
    assert_eq!(wire.upstream.as_deref(), Some("http://u"));
    assert!(wire.responses);
}

fn stop_hook_args(extra: &[&str]) -> Result<WorkerStopHookArgs, clap::Error> {
    let base = [
        "ralph",
        "worker-stop-hook",
        "--reported",
        "R",
        "--unreported",
        "U",
    ];
    let cli = Cli::try_parse_from(base.iter().chain(extra))?;
    match cli.command {
        Some(Commands::WorkerStopHook(args)) => Ok(args),
        other => panic!("expected worker-stop-hook, got {other:?}"),
    }
}

#[test]
fn test_cli_stop_hook_usage_requires_usage_seen() {
    assert!(stop_hook_args(&["--usage", "/u"]).is_err());
}

#[test]
fn test_cli_stop_hook_usage_seen_does_not_require_usage() {
    let args = stop_hook_args(&["--worker", "X", "--usage-seen", "P"]).unwrap();
    assert_eq!(args.usage, None);
}

#[test]
fn test_cli_stop_hook_takes_usage_with_usage_seen() {
    let args = stop_hook_args(&["--worker", "X", "--usage-seen", "P", "--usage", "/u"]).unwrap();
    assert_eq!(args.usage, Some(std::path::PathBuf::from("/u")));
}

#[test]
fn test_cli_chat_history_takes_a_scope() {
    let Some(Commands::Chat(chat)) = Cli::try_parse_from([
        "ralph", "chat", "--thread", "t1", "history", "recall", "x", "--in", "main",
    ])
    .unwrap()
    .command
    else {
        panic!("expected chat");
    };
    let Some(ChatCommand::History(history)) = chat.command else {
        panic!("expected chat history");
    };
    assert_eq!(history.scope.as_deref(), Some("main"));
}

#[test]
fn test_cli_chat_history_takes_a_tool_words_cursor_and_store() {
    let Some(Commands::Chat(chat)) = Cli::try_parse_from([
        "ralph", "chat", "history", "search", "rate", "limit", "--cursor", "c1", "--store", "/s",
    ])
    .unwrap()
    .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.store, Some(std::path::PathBuf::from("/s")));
    let Some(ChatCommand::History(history)) = chat.command else {
        panic!("expected chat history");
    };
    assert_eq!(history.tool, HistoryTool::Search);
    assert_eq!(history.words, ["rate", "limit"]);
    assert_eq!(history.cursor.as_deref(), Some("c1"));
    assert_eq!(history.scope, None);
    let Some(Commands::Chat(chat)) =
        Cli::try_parse_from(["ralph", "chat", "--store", "/s", "history", "zoom", "4-7"])
            .unwrap()
            .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.store, Some(std::path::PathBuf::from("/s")));
    assert!(Cli::try_parse_from(["ralph", "chat", "history", "zoom"]).is_err());
    assert!(Cli::try_parse_from(["ralph", "chat", "history", "drop", "1"]).is_err());
}

#[test]
fn test_cli_upgrade_takes_check_and_force() {
    let Some(Commands::Upgrade(upgrade)) =
        Cli::try_parse_from(["ralph", "upgrade", "--check", "--force"])
            .unwrap()
            .command
    else {
        panic!("expected upgrade");
    };
    assert!(upgrade.check && upgrade.force);
}

#[test]
fn test_cli_chat_thread_takes_a_short_lowercase_name() {
    let Some(Commands::Chat(chat)) = Cli::try_parse_from(["ralph", "chat", "--thread", "auth"])
        .unwrap()
        .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.thread.map(|t| t.to_string()).as_deref(), Some("auth"));
    for bad in ["Auth", "main", &"a".repeat(33)] {
        assert!(
            Cli::try_parse_from(["ralph", "chat", "--thread", bad]).is_err(),
            "{bad}"
        );
    }
    let Some(Commands::Chat(chat)) = Cli::try_parse_from([
        "ralph", "chat", "history", "zoom", "auth:0", "--thread", "auth",
    ])
    .unwrap()
    .command
    else {
        panic!("expected chat");
    };
    assert!(chat.thread.is_some());
}

#[test]
fn test_cli_thread_ls_and_drop_take_a_store() {
    let Some(Commands::Thread(thread)) = Cli::try_parse_from(["ralph", "thread", "ls"])
        .unwrap()
        .command
    else {
        panic!("expected thread");
    };
    assert!(matches!(thread.command, ThreadCommand::Ls));
    let Some(Commands::Thread(thread)) =
        Cli::try_parse_from(["ralph", "thread", "drop", "auth", "--store", "/s"])
            .unwrap()
            .command
    else {
        panic!("expected thread");
    };
    assert_eq!(thread.store, Some(std::path::PathBuf::from("/s")));
    let ThreadCommand::Drop { name } = thread.command else {
        panic!("expected drop");
    };
    assert_eq!(name.as_str(), "auth");
    assert!(Cli::try_parse_from(["ralph", "thread", "drop", "Bad"]).is_err());
}

#[test]
fn test_cli_thread_park_and_revive_take_a_name() {
    for (verb, park) in [("park", true), ("revive", false)] {
        let Some(Commands::Thread(thread)) = Cli::try_parse_from(["ralph", "thread", verb, "auth"])
            .unwrap()
            .command
        else {
            panic!("expected thread");
        };
        let name = match (thread.command, park) {
            (ThreadCommand::Park { name }, true) | (ThreadCommand::Revive { name }, false) => name,
            (other, _) => panic!("{verb} parsed as {other:?}"),
        };
        assert_eq!(name.as_str(), "auth");
        assert!(Cli::try_parse_from(["ralph", "thread", verb, "Bad"]).is_err());
    }
}

#[test]
fn test_cli_thread_merge_takes_rebase() {
    for (args, rebase) in [
        (&["ralph", "thread", "merge", "auth"][..], false),
        (&["ralph", "thread", "merge", "auth", "--rebase"][..], true),
    ] {
        let Some(Commands::Thread(thread)) = Cli::try_parse_from(args).unwrap().command else {
            panic!("expected thread");
        };
        let ThreadCommand::Merge { name, rebase: got } = thread.command else {
            panic!("expected merge");
        };
        assert_eq!((name.as_str(), got), ("auth", rebase), "{args:?}");
    }
}

#[test]
fn test_cli_thread_pull_takes_a_name() {
    let Some(Commands::Thread(thread)) = Cli::try_parse_from(["ralph", "thread", "pull", "auth"])
        .unwrap()
        .command
    else {
        panic!("expected thread");
    };
    let ThreadCommand::Pull { name } = thread.command else {
        panic!("expected pull");
    };
    assert_eq!(name.as_str(), "auth");
    assert!(Cli::try_parse_from(["ralph", "thread", "pull", "Bad"]).is_err());
}

#[test]
fn test_cli_inbox_post_takes_store_from_and_input() {
    let Some(Commands::Inbox(inbox)) = Cli::try_parse_from([
        "ralph",
        "inbox",
        "post",
        "--store",
        "S",
        "--from",
        "ci",
        "report.txt",
    ])
    .unwrap()
    .command
    else {
        panic!("expected inbox");
    };
    let InboxCommand::Post { store, from, input } = inbox.command;
    assert_eq!(store, PathBuf::from("S"));
    assert_eq!(from, "ci");
    assert_eq!(input, PathBuf::from("report.txt"));
}

#[test]
fn test_cli_inbox_post_takes_dash_for_stdin() {
    let Some(Commands::Inbox(inbox)) = Cli::try_parse_from([
        "ralph", "inbox", "post", "--store", "S", "--from", "ci", "-",
    ])
    .unwrap()
    .command
    else {
        panic!("expected inbox");
    };
    let InboxCommand::Post { input, .. } = inbox.command;
    assert_eq!(input, PathBuf::from("-"));
}

#[test]
fn test_cli_inbox_post_requires_store_from_and_input() {
    for args in [
        vec!["ralph", "inbox", "post", "--from", "ci", "-"],
        vec!["ralph", "inbox", "post", "--store", "S", "-"],
        vec!["ralph", "inbox", "post", "--store", "S", "--from", "ci"],
    ] {
        assert!(Cli::try_parse_from(&args).is_err(), "{args:?}");
    }
}

#[test]
fn test_cli_inbox_post_rejects_a_bad_source() {
    for bad in ["external:x", "Ci", "-x", "", &"a".repeat(33)] {
        assert!(
            Cli::try_parse_from(["ralph", "inbox", "post", "--store", "S", "--from", bad, "-"])
                .is_err(),
            "{bad}"
        );
    }
}
