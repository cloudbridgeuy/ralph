use super::*;

#[test]
fn test_cli_chat_model_needs_a_thread_and_a_valid_value() {
    let Some(Commands::Chat(chat)) =
        Cli::try_parse_from(["ralph", "chat", "--thread", "auth", "--model", "haiku"])
            .unwrap()
            .command
    else {
        panic!("expected chat");
    };
    assert_eq!(chat.model.as_deref(), Some("haiku"));
    for args in [
        &["ralph", "chat", "--model", "haiku"][..],
        &["ralph", "chat", "--thread", "auth", "--model", "gpt-5"][..],
        &["ralph", "chat", "--thread", "auth", "--model"][..],
    ] {
        assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
    }
    let err = Cli::try_parse_from(["ralph", "chat", "--model", "haiku"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
}
