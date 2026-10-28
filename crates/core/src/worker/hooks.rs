use super::{usage_seen_path, WorkerMarkers, REPORT_REMINDER};
use crate::snapshot::WorkerSite;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn stop_hook_command(
    exe: &Path,
    markers: &WorkerMarkers,
    site: Option<&WorkerSite>,
    usage: &UsageSink,
) -> String {
    let mut words = vec![
        shell_quote(&exe.to_string_lossy()),
        "worker-stop-hook".to_string(),
        "--reported".to_string(),
        shell_quote(&markers.reported.to_string_lossy()),
        "--unreported".to_string(),
        shell_quote(&markers.unreported.to_string_lossy()),
        "--worker".to_string(),
        shell_quote(usage.worker),
        "--usage".to_string(),
        shell_quote(&usage.usage.to_string_lossy()),
        "--usage-seen".to_string(),
        shell_quote(&usage_seen_path(usage.store, usage.worker).to_string_lossy()),
    ];
    words.extend(
        site.into_iter()
            .flat_map(WorkerSite::args)
            .map(|arg| shell_quote(&arg)),
    );
    words.join(" ")
}

#[derive(Debug, Clone, Copy)]
pub struct UsageSink<'a> {
    pub store: &'a Path,
    pub worker: &'a str,
    pub usage: &'a Path,
}

fn touch_command(path: &Path) -> String {
    format!("touch {}", shell_quote(&path.to_string_lossy()))
}

fn command_hook(command: String) -> Value {
    json!({"type": "command", "command": command})
}

pub fn claude_worker_settings(
    exe: &Path,
    markers: &WorkerMarkers,
    site: Option<&WorkerSite>,
    usage: &UsageSink,
) -> Value {
    let quote = |path: &Path| shell_quote(&path.to_string_lossy());
    let clear_blocked = format!("rm -f {}", quote(&markers.blocked));
    let idle = format!(
        "{clear_blocked}; [ ! -e {} ] || {}",
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
                "hooks": [
                    command_hook(stop_hook_command(exe, markers, site, usage)),
                    command_hook(clear_blocked)
                ]
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

#[derive(Deserialize)]
pub struct StopSession {
    pub session_id: String,
    pub transcript_path: std::path::PathBuf,
}

pub fn parse_stop_session(payload: &str) -> Result<StopSession, String> {
    serde_json::from_str(payload).map_err(|e| format!("Stop hook input has no transcript: {e}"))
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

    fn sink() -> UsageSink<'static> {
        UsageSink {
            store: Path::new("/s"),
            worker: "A",
            usage: Path::new("/u/usage.jsonl"),
        }
    }

    fn site() -> WorkerSite {
        WorkerSite {
            store: "/s".into(),
            id: "A".into(),
            worktree: "/w".into(),
        }
    }

    #[test]
    fn claude_worker_settings_touch_the_ready_file_on_startup_only() {
        let markers = WorkerMarkers::new(Path::new("/s/it's"), "A");
        let settings =
            claude_worker_settings(Path::new("/bin/ralph"), &markers, Some(&site()), &sink());
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
        let settings =
            claude_worker_settings(Path::new("/bin/ral'ph"), &markers, Some(&site()), &sink());
        assert_eq!(
            hook_command(&settings, "Stop"),
            r"'/bin/ral'\''ph' worker-stop-hook --reported '/s/it'\''s/reported/A' --unreported '/s/it'\''s/unreported/A' --worker 'A' --usage '/u/usage.jsonl' --usage-seen '/s/workers/A.usage-seen' '--store' '/s' '--id' 'A' '--worktree' '/w'"
        );
    }

    #[test]
    fn claude_stop_hook_omits_the_site_arguments_without_a_site() {
        let settings = claude_worker_settings(Path::new("/bin/ralph"), &markers(), None, &sink());
        assert_eq!(
            hook_command(&settings, "Stop"),
            "'/bin/ralph' worker-stop-hook --reported '/s/reported/A' --unreported '/s/unreported/A' --worker 'A' --usage '/u/usage.jsonl' --usage-seen '/s/workers/A.usage-seen'"
        );
    }

    #[test]
    fn claude_permission_hook_only_touches_the_blocked_marker() {
        let settings =
            claude_worker_settings(Path::new("/bin/ralph"), &markers(), Some(&site()), &sink());
        assert_eq!(
            hook_command(&settings, "PermissionRequest"),
            "touch '/s/blocked/A'"
        );
    }

    #[test]
    fn claude_idle_notification_clears_blocked_and_touches_waiting_only_while_unreported() {
        let settings =
            claude_worker_settings(Path::new("/bin/ralph"), &markers(), Some(&site()), &sink());
        assert_eq!(
            settings["hooks"]["Notification"][0]["matcher"],
            "idle_prompt"
        );
        assert_eq!(
            hook_command(&settings, "Notification"),
            "rm -f '/s/blocked/A'; [ ! -e '/s/unreported/A' ] || touch '/s/waiting/A'"
        );
    }

    #[test]
    fn claude_stop_clears_the_blocked_marker_after_the_stop_hook() {
        let settings =
            claude_worker_settings(Path::new("/bin/ralph"), &markers(), Some(&site()), &sink());
        assert_eq!(
            settings["hooks"]["Stop"][0]["hooks"][1]["command"],
            "rm -f '/s/blocked/A'"
        );
    }

    #[test]
    fn claude_activity_hooks_clear_blocked_and_waiting_for_every_event() {
        let settings =
            claude_worker_settings(Path::new("/bin/ralph"), &markers(), Some(&site()), &sink());
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
    fn stop_session_reads_the_session_and_transcript() {
        let session =
            parse_stop_session(r#"{"session_id":"s1","transcript_path":"/t/s1.jsonl","x":1}"#)
                .unwrap();
        assert_eq!(session.session_id, "s1");
        assert_eq!(session.transcript_path, Path::new("/t/s1.jsonl"));
        assert!(parse_stop_session("{}").is_err());
    }

    #[test]
    fn stop_block_output_carries_the_reminder() {
        assert_eq!(
            stop_block_output(),
            json!({"decision": "block", "reason": REPORT_REMINDER})
        );
    }
}
