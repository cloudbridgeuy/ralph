use super::*;
use crate::worker::CURSOR_VERIFY_HOOKS;
use serde_json::{json, Value};

const BEFORE_SHELL: &str = include_str!("fixtures/before_shell.json");

fn ours() -> String {
    bridge_hook_command(Path::new("/bin/ralph"), Path::new("/run/w1/hook.sock"))
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
    let verify_command = parsed(CURSOR_VERIFY_HOOKS)["hooks"]["beforeShellExecution"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        parsed(&add_bridge_hook(None, &verify_command)),
        parsed(CURSOR_VERIFY_HOOKS)
    );
    assert_eq!(
        parsed(&add_bridge_hook(None, &ours())),
        json!({"version": 1, "hooks": {"beforeShellExecution": [{"command": ours()}]}})
    );
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
        json!([{"command": "user-check.sh"}, {"command": ours()}])
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
        json!([{"command": ours()}])
    );
}

#[test]
fn adding_replaces_an_unreadable_file() {
    for existing in ["not json", "[]", "null", r#"{"hooks": []}"#] {
        assert_eq!(
            shell_hooks(&add_bridge_hook(Some(existing), &ours())),
            json!([{"command": ours()}]),
            "{existing}"
        );
    }
    let bad_event = r#"{"version": 1, "hooks": {"beforeShellExecution": {}}}"#;
    assert_eq!(
        shell_hooks(&add_bridge_hook(Some(bad_event), &ours())),
        json!([{"command": ours()}])
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
