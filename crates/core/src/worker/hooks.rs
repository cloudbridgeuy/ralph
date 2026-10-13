use super::{opencode_session_id, WorkerMarkers, REPORT_REMINDER};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn stop_hook_command(exe: &Path, markers: &WorkerMarkers) -> String {
    format!(
        "{} worker-stop-hook --reported {} --unreported {}",
        shell_quote(&exe.to_string_lossy()),
        shell_quote(&markers.reported.to_string_lossy()),
        shell_quote(&markers.unreported.to_string_lossy())
    )
}

fn touch_command(path: &Path) -> String {
    format!("touch {}", shell_quote(&path.to_string_lossy()))
}

fn command_hook(command: String) -> Value {
    json!({"type": "command", "command": command})
}

pub fn claude_worker_settings(exe: &Path, markers: &WorkerMarkers) -> Value {
    let quote = |path: &Path| shell_quote(&path.to_string_lossy());
    let idle = format!(
        "[ ! -e {} ] || {}",
        quote(&markers.unreported),
        touch_command(&markers.waiting)
    );
    let clear = format!(
        "rm -f {} {}",
        quote(&markers.blocked),
        quote(&markers.waiting)
    );
    json!({
        "hooks": {
            "SessionStart": [{
                "matcher": "startup",
                "hooks": [command_hook(touch_command(&markers.ready))]
            }],
            "Stop": [{
                "hooks": [command_hook(stop_hook_command(exe, markers))]
            }],
            "PermissionRequest": [{
                "hooks": [command_hook(touch_command(&markers.blocked))]
            }],
            "Notification": [{
                "matcher": "idle_prompt",
                "hooks": [command_hook(idle)]
            }],
            "UserPromptSubmit": [{
                "hooks": [command_hook(clear.clone())]
            }],
            "PostToolUse": [{
                "hooks": [command_hook(clear)]
            }]
        }
    })
}

#[derive(Deserialize)]
struct StopPayload {
    stop_hook_active: Option<bool>,
    background_tasks: Option<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopDecision {
    AllowBackground,
    GiveUp,
    ConsumeMarker,
    Block,
    FailOpen(String),
}

pub fn stop_decision(payload: &str, reported: bool) -> StopDecision {
    let payload: StopPayload = match serde_json::from_str(payload) {
        Ok(payload) => payload,
        Err(e) => return StopDecision::FailOpen(format!("unreadable Stop hook input: {e}")),
    };
    if payload
        .background_tasks
        .is_some_and(|tasks| !tasks.is_empty())
    {
        return StopDecision::AllowBackground;
    }
    if reported {
        return StopDecision::ConsumeMarker;
    }
    match payload.stop_hook_active {
        Some(false) => StopDecision::Block,
        Some(true) => StopDecision::GiveUp,
        None => StopDecision::FailOpen("Stop hook input has no stop_hook_active".to_string()),
    }
}

pub fn stop_block_output() -> Value {
    json!({"decision": "block", "reason": REPORT_REMINDER})
}

pub fn opencode_worker_plugin(id: &str, ready: &Path, reported: &Path) -> String {
    let session = Value::from(opencode_session_id(id));
    let ready = Value::from(ready.to_string_lossy());
    let reported = Value::from(reported.to_string_lossy());
    let reminder = Value::from(REPORT_REMINDER);
    let plugin = Value::from(format!("ralph-worker-{id}"));
    format!(
        r#"import {{ existsSync, rmSync, writeFileSync }} from "node:fs"

const SESSION = {session}
const READY = {ready}
const REPORTED = {reported}
const REMINDER = {reminder}
const STARTED = new Set(["session.created", "session.inbox.enqueued"])
const ENDED = new Set(["session.execution.succeeded", "session.execution.failed"])

export default {{
  id: {plugin},
  setup: async (ctx: any) => {{
    let ready = false
    let nudged = false
    void (async () => {{
      try {{
        for await (const event of ctx.event.subscribe({{}})) {{
          if (event?.data?.sessionID !== SESSION) continue
          if (!ready && STARTED.has(event.type)) {{
            ready = true
            writeFileSync(READY, "")
          }}
          if (!ENDED.has(event.type)) continue
          if (existsSync(REPORTED)) {{
            rmSync(REPORTED, {{ force: true }})
            nudged = false
          }} else if (nudged) {{
            nudged = false
          }} else {{
            nudged = true
            ctx.session.prompt({{ sessionID: SESSION, text: REMINDER }}).catch(() => {{}})
          }}
        }}
      }} catch {{}}
    }})()
  }},
}} as any
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook_command(settings: &Value, event: &str) -> String {
        settings["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn markers() -> WorkerMarkers {
        WorkerMarkers::new(Path::new("/s"), "A")
    }

    #[test]
    fn claude_worker_settings_touch_the_ready_file_on_startup_only() {
        let markers = WorkerMarkers::new(Path::new("/s/it's"), "A");
        let settings = claude_worker_settings(Path::new("/bin/ralph"), &markers);
        let start = &settings["hooks"]["SessionStart"][0];
        assert_eq!(start["matcher"], "startup");
        assert_eq!(
            start["hooks"][0],
            json!({"type": "command", "command": r"touch '/s/it'\''s/ready/A'"})
        );
    }

    #[test]
    fn claude_stop_hook_runs_ralph_with_the_marker() {
        let markers = WorkerMarkers::new(Path::new("/s/it's"), "A");
        let settings = claude_worker_settings(Path::new("/bin/ral'ph"), &markers);
        assert_eq!(
            hook_command(&settings, "Stop"),
            r"'/bin/ral'\''ph' worker-stop-hook --reported '/s/it'\''s/reported/A' --unreported '/s/it'\''s/unreported/A'"
        );
    }

    #[test]
    fn claude_permission_hook_only_touches_the_blocked_marker() {
        let settings = claude_worker_settings(Path::new("/bin/ralph"), &markers());
        assert_eq!(
            hook_command(&settings, "PermissionRequest"),
            "touch '/s/blocked/A'"
        );
    }

    #[test]
    fn claude_idle_notification_touches_waiting_only_while_unreported() {
        let settings = claude_worker_settings(Path::new("/bin/ralph"), &markers());
        assert_eq!(
            settings["hooks"]["Notification"][0]["matcher"],
            "idle_prompt"
        );
        assert_eq!(
            hook_command(&settings, "Notification"),
            "[ ! -e '/s/unreported/A' ] || touch '/s/waiting/A'"
        );
    }

    #[test]
    fn claude_activity_hooks_clear_blocked_and_waiting_for_every_event() {
        let settings = claude_worker_settings(Path::new("/bin/ralph"), &markers());
        for event in ["UserPromptSubmit", "PostToolUse"] {
            assert!(
                settings["hooks"][event][0].get("matcher").is_none(),
                "{event}"
            );
            assert_eq!(
                hook_command(&settings, event),
                "rm -f '/s/blocked/A' '/s/waiting/A'",
                "{event}"
            );
        }
    }

    #[test]
    fn stop_decision_blocks_only_an_unreported_first_stop() {
        assert_eq!(
            stop_decision(r#"{"stop_hook_active":false}"#, false),
            StopDecision::Block
        );
        assert_eq!(
            stop_decision(r#"{"stop_hook_active":true}"#, false),
            StopDecision::GiveUp
        );
        assert_eq!(
            stop_decision(r#"{"stop_hook_active":false}"#, true),
            StopDecision::ConsumeMarker
        );
        assert_eq!(
            stop_decision(r#"{"stop_hook_active":true}"#, true),
            StopDecision::ConsumeMarker
        );
    }

    #[test]
    fn stop_decision_reads_fields_whatever_the_layout() {
        let spaced = "{\n  \"last_assistant_message\" : \"say \\\"stop_hook_active\\\": true\",\n  \"background_tasks\" : [ ],\n  \"stop_hook_active\" :  false\n}";
        assert_eq!(stop_decision(spaced, false), StopDecision::Block);
        let nested = r#"{"tool_input":{"stop_hook_active":true},"stop_hook_active":false}"#;
        assert_eq!(stop_decision(nested, false), StopDecision::Block);
    }

    #[test]
    fn stop_decision_waits_for_background_tasks_and_keeps_the_marker() {
        let busy = r#"{"stop_hook_active":false,"background_tasks":[{"id":"x"}]}"#;
        assert_eq!(stop_decision(busy, false), StopDecision::AllowBackground);
        assert_eq!(stop_decision(busy, true), StopDecision::AllowBackground);
        let idle = r#"{"stop_hook_active":false,"background_tasks":null}"#;
        assert_eq!(stop_decision(idle, false), StopDecision::Block);
    }

    #[test]
    fn stop_decision_fails_open_on_bad_input() {
        for payload in ["", "not json", r#"{"stop_hook_active":"no"}"#, "{}"] {
            assert!(
                matches!(stop_decision(payload, false), StopDecision::FailOpen(_)),
                "{payload}"
            );
        }
        assert!(matches!(
            stop_decision("not json", true),
            StopDecision::FailOpen(_)
        ));
        assert_eq!(stop_decision("{}", true), StopDecision::ConsumeMarker);
    }

    #[test]
    fn stop_block_output_carries_the_reminder() {
        assert_eq!(
            stop_block_output(),
            json!({"decision": "block", "reason": REPORT_REMINDER})
        );
    }

    #[test]
    fn opencode_worker_plugin_answers_only_for_its_own_session() {
        let plugin =
            opencode_worker_plugin("A", Path::new("/s/ready/\"A\""), Path::new("/s/reported/A"));
        assert!(plugin.contains(r#"const SESSION = "ses_ralphA""#));
        assert!(plugin.contains(r#"const READY = "/s/ready/\"A\"""#));
        assert!(plugin.contains(r#"const REPORTED = "/s/reported/A""#));
        assert!(plugin.contains(&format!(
            "const REMINDER = {}",
            Value::from(REPORT_REMINDER)
        )));
        assert!(plugin.contains(r#"id: "ralph-worker-A""#));
        assert!(plugin.contains(r#"["session.created", "session.inbox.enqueued"]"#));
        assert!(plugin.contains(r#"["session.execution.succeeded", "session.execution.failed"]"#));
        assert!(plugin.contains("if (event?.data?.sessionID !== SESSION) continue"));
        for forbidden in ["setInterval", "setTimeout", "child_process"] {
            assert!(!plugin.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn opencode_worker_plugin_nudges_once_unless_the_worker_reported() {
        let plugin = opencode_worker_plugin("A", Path::new("/r"), Path::new("/m"));
        let marker = plugin.find("if (existsSync(REPORTED))").unwrap();
        let guard = plugin
            .find("} else if (nudged) {\n            nudged = false")
            .unwrap();
        let nudge = plugin
            .find("nudged = true\n            ctx.session.prompt({ sessionID: SESSION, text: REMINDER })")
            .unwrap();
        assert!(marker < guard && guard < nudge);
    }
}
