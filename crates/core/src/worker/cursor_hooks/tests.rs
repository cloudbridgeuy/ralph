use super::*;
use crate::chat_socket::{check_socket_path, runtime_dir_pid, runtime_root};
use crate::worker::CURSOR_VERIFY_HOOKS;
use serde_json::{json, Value};

const BEFORE_SHELL: &str = include_str!("fixtures/before_shell.json");

fn ours() -> String {
    bridge_hook_command(Path::new("/bin/ralph"), Path::new("/run/w1/hook.sock"))
}

fn entry() -> Value {
    json!({"command": ours(), "timeout": 150, "failClosed": true})
}

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn shell_hooks(text: &str) -> Value {
    parsed(text)["hooks"]["beforeShellExecution"].clone()
}

#[test]
fn the_bridge_command_quotes_both_paths() {
    let command = bridge_hook_command(Path::new("/a b/ralph"), Path::new("/run/it's/hook.sock"));
    assert_eq!(
        command,
        r"'/a b/ralph' hook-bridge --socket '/run/it'\''s/hook.sock'"
    );
}

#[test]
fn adding_to_no_file_builds_the_verify_hook_shape() {
    let built = parsed(&add_bridge_hook(None, &ours()));
    assert_eq!(built["version"], parsed(CURSOR_VERIFY_HOOKS)["version"]);
    assert_eq!(
        built,
        json!({"version": 1, "hooks": {"beforeShellExecution": [entry()]}})
    );
}

#[test]
fn cursor_fails_closed_after_the_bridge_timeout() {
    let built = shell_hooks(&add_bridge_hook(None, &ours()));
    assert_eq!(built[0]["failClosed"], true);
    let cursor_secs = built[0]["timeout"].as_u64().unwrap_or_default();
    assert!(cursor_secs > BRIDGE_TIMEOUT.as_secs(), "{cursor_secs}");
}

#[test]
fn adding_keeps_other_entries_events_and_keys() {
    let existing = json!({
        "version": 1,
        "extra": true,
        "hooks": {
            "beforeShellExecution": [{"command": "user-check.sh"}],
            "afterFileEdit": [{"command": "fmt.sh"}]
        }
    })
    .to_string();
    let merged = parsed(&add_bridge_hook(Some(&existing), &ours()));
    assert_eq!(merged["extra"], true);
    assert_eq!(
        merged["hooks"]["afterFileEdit"],
        json!([{"command": "fmt.sh"}])
    );
    assert_eq!(
        merged["hooks"]["beforeShellExecution"],
        json!([{"command": "user-check.sh"}, entry()])
    );
}

#[test]
fn adding_again_or_over_a_stale_bridge_keeps_one_bridge_entry() {
    let once = add_bridge_hook(None, &ours());
    assert_eq!(
        parsed(&add_bridge_hook(Some(&once), &ours())),
        parsed(&once)
    );
    let stale = add_bridge_hook(
        None,
        &bridge_hook_command(Path::new("/old/ralph"), Path::new("/run/w0/hook.sock")),
    );
    assert_eq!(
        shell_hooks(&add_bridge_hook(Some(&stale), &ours())),
        json!([entry()])
    );
}

#[test]
fn adding_replaces_an_unreadable_file() {
    for existing in ["not json", "[]", "null", r#"{"hooks": []}"#] {
        assert_eq!(
            shell_hooks(&add_bridge_hook(Some(existing), &ours())),
            json!([entry()]),
            "{existing}"
        );
    }
    let bad_event = r#"{"version": 1, "hooks": {"beforeShellExecution": {}}}"#;
    assert_eq!(
        shell_hooks(&add_bridge_hook(Some(bad_event), &ours())),
        json!([entry()])
    );
}

#[test]
fn removing_takes_out_only_bridge_entries() {
    let existing = json!({
        "version": 1,
        "hooks": {
            "beforeShellExecution": [{"command": "user-check.sh"}, {"command": ours()}],
            "afterFileEdit": [{"command": "fmt.sh"}]
        }
    })
    .to_string();
    let removed = parsed(&remove_bridge_hook(&existing));
    assert_eq!(
        removed["hooks"]["beforeShellExecution"],
        json!([{"command": "user-check.sh"}])
    );
    assert_eq!(
        removed["hooks"]["afterFileEdit"],
        json!([{"command": "fmt.sh"}])
    );
}

#[test]
fn removing_the_last_bridge_entry_drops_the_event() {
    let removed = parsed(&remove_bridge_hook(&add_bridge_hook(None, &ours())));
    assert_eq!(removed, json!({"version": 1, "hooks": {}}));
}

#[test]
fn removing_leaves_a_file_without_a_bridge_unchanged() {
    for existing in ["not json", r#"{"hooks": {"stop": [{"command": "x"}]}}"#] {
        assert_eq!(remove_bridge_hook(existing), existing);
    }
}

#[test]
fn the_measured_stdin_parses() {
    let input = parse_hook_input(BEFORE_SHELL).unwrap();
    assert_eq!(input.hook_event_name, HookEvent::BeforeShellExecution);
    assert_eq!(input.command, "echo hi");
    assert_eq!(input.cwd, "");
    assert_eq!(input.session_id, "2620ed41-1c06-4377-bbf8-34b1fb0b8fd6");
    assert_eq!(input.conversation_id, input.session_id);
    assert_eq!(input.generation_id, input.session_id);
    assert_eq!(input.tool_use_id, "fa1e5238-8f83-4e97-b7ce-65b058f843b6");
    assert_eq!(
        input.workspace_roots,
        vec![PathBuf::from("/tmp/guz1495-hookcheck")]
    );
    assert_eq!(input.model, "grok-4.5");
    assert_eq!(input.cursor_version, "2026.10.22-27dcf5f");
}

#[test]
fn optional_stdin_fields_default_to_empty() {
    let input =
        parse_hook_input(r#"{"hook_event_name":"beforeShellExecution","command":"ls","new":1}"#)
            .unwrap();
    assert_eq!(input.command, "ls");
    assert!(input.session_id.is_empty() && input.workspace_roots.is_empty());
}

#[test]
fn unusable_stdin_is_an_error() {
    for text in [
        "",
        "not json",
        r#"{"hook_event_name":"beforeShellExecution"}"#,
        r#"{"hook_event_name":"beforeMCPExecution","command":"ls"}"#,
        r#"{"command":"ls"}"#,
    ] {
        assert!(parse_hook_input(text).is_err(), "{text}");
    }
}

fn output(outcome: HookOutcome) -> Value {
    parsed(&hook_output(&outcome))
}

#[test]
fn the_hook_offers_allow_once_and_deny() {
    let ids: Vec<(String, String)> = hook_options()
        .into_iter()
        .map(|option| (option.option_id, option.kind))
        .collect();
    assert_eq!(
        ids,
        vec![
            ("allow_once".to_string(), "allow_once".to_string()),
            ("deny".to_string(), "reject_once".to_string())
        ]
    );
}

#[test]
fn allow_once_allows() {
    assert_eq!(
        output(HookOutcome::Answered(ALLOW_ONCE.to_string())),
        json!({"permission": "allow"})
    );
}

#[test]
fn every_other_outcome_is_an_explicit_deny() {
    let outcomes = [
        (HookOutcome::Answered("deny".into()), "the person denied"),
        (HookOutcome::Answered("other".into()), "the person denied"),
        (HookOutcome::TimedOut, "no answer within 120 s"),
        (HookOutcome::BridgeGone("no socket".into()), "no socket"),
        (HookOutcome::BadInput("bad json".into()), "bad json"),
    ];
    for (outcome, reason) in outcomes {
        let value = output(outcome.clone());
        assert_eq!(value["permission"], "deny", "{outcome:?}");
        for key in ["user_message", "agent_message"] {
            let message = value[key].as_str().unwrap_or_default();
            assert!(message.contains(reason), "{outcome:?} {key}: {message}");
        }
    }
}

#[test]
fn the_bridge_waits_two_minutes() {
    assert_eq!(BRIDGE_TIMEOUT, Duration::from_secs(120));
}

#[test]
fn the_request_id_is_the_tool_use_id_or_a_counter() {
    assert_eq!(hook_request_id("fa1e5238", 7), "fa1e5238");
    assert_eq!(hook_request_id("", 7), "hook-7");
    assert_eq!(hook_request_id("  ", 8), "hook-8");
}

#[test]
fn an_answer_line_is_the_chosen_option() {
    let line = crate::chat_socket::encode_line(&BridgeAnswer {
        option_id: ALLOW_ONCE.to_string(),
    });
    assert_eq!(
        read_outcome(Ok(line)),
        HookOutcome::Answered(ALLOW_ONCE.to_string())
    );
}

#[test]
fn a_read_timeout_is_timed_out() {
    for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut] {
        assert_eq!(
            read_outcome(Err(io::Error::from(kind))),
            HookOutcome::TimedOut,
            "{kind:?}"
        );
    }
}

#[test]
fn a_closed_refused_or_garbled_bridge_is_gone() {
    let gone = |read: io::Result<String>| matches!(read_outcome(read), HookOutcome::BridgeGone(_));
    assert!(gone(Ok(String::new())));
    assert!(gone(Ok("not json\n".to_string())));
    assert!(gone(Err(io::Error::from(io::ErrorKind::NotFound))));
    assert!(gone(Err(io::Error::from(io::ErrorKind::ConnectionRefused))));
}

#[test]
fn the_ask_round_trips_as_one_line() {
    let ask = BridgeAsk {
        tool_use_id: "t1".to_string(),
        command: "echo hi".to_string(),
    };
    let line = crate::chat_socket::encode_line(&ask);
    assert_eq!(line.matches('\n').count(), 1);
    assert_eq!(serde_json::from_str::<BridgeAsk>(&line).unwrap(), ask);
}

#[test]
fn the_worker_gives_up_at_the_bridge_timeout_or_when_the_hook_leaves() {
    let just_before = BRIDGE_TIMEOUT - Duration::from_millis(1);
    assert_eq!(bridge_wait(Duration::ZERO, true), BridgeWait::Wait);
    assert_eq!(bridge_wait(just_before, true), BridgeWait::Wait);
    assert_eq!(bridge_wait(BRIDGE_TIMEOUT, true), BridgeWait::GiveUp);
    assert_eq!(bridge_wait(Duration::ZERO, false), BridgeWait::GiveUp);
}

#[test]
fn a_hook_ask_is_a_shell_permission_request() {
    let ask = BridgeAsk {
        tool_use_id: "t1".to_string(),
        command: "rm -rf build".to_string(),
    };
    assert_eq!(
        hook_content("t1", &ask),
        BlockedContent {
            request_id: "t1".to_string(),
            tool: "shell".to_string(),
            summary: "rm -rf build".to_string(),
            options: hook_options(),
            pending: Vec::new(),
        }
    );
}

#[test]
fn the_hook_socket_fits_the_unix_limit_for_a_long_store() {
    let store = PathBuf::from(format!("/{}", "deep/".repeat(60)));
    let key = StoreKey::of(&store);
    let root = runtime_root(u32::MAX);
    let name = hook_socket_name(&key, u32::MAX, u64::MAX);
    let path = root.join(&name);
    assert!(path.as_os_str().len() < 104, "{}", path.display());
    assert_eq!(check_socket_path(&root, &path), Ok(()));
    assert!(name.ends_with(".sock"), "{name}");
    assert_ne!(hook_socket_name(&key, 7, 1), hook_socket_name(&key, 7, 2));
    assert_eq!(runtime_dir_pid(&key, &name), None);
}
