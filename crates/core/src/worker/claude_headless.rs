use super::acp::REPORT_TOOL;
use super::{
    BlockedContent, PermissionMode, PermissionOption, ThreadSignal, TurnEnd, VersionGate,
    SERVER_NAME,
};
use crate::stream::result_is_error;
use serde_json::{json, Value};
use std::path::Path;

pub const CLAUDE_COMMAND: &str = "claude";
pub const CLAUDE_HEADLESS_SPAWN_DIR: &str = "claude-headless-spawn";
pub const CLAUDE_HEADLESS_CLOSE_DIR: &str = "claude-headless-close";
pub const CLAUDE_HEADLESS_ERROR_DIR: &str = "claude-headless-error";
pub const CLAUDE_HEADLESS_OWNER_DIR: &str = "claude-headless";
pub const CLAUDE_HEADLESS_ANSWER_DIR: &str = "claude-headless-answer";
pub const CLAUDE_HEADLESS_PROMPT_DIR: &str = "claude-headless-prompt";
pub const CLAUDE_HEADLESS_INTERRUPT_DIR: &str = "claude-headless-interrupt";
pub const CLAUDE_HEADLESS_INTERRUPT_RESULT_DIR: &str = "claude-headless-interrupt-result";
pub const CLAUDE_HEADLESS_SESSION_DIR: &str = "claude-headless-session";

pub const ALLOW_ONCE: &str = "allow_once";
const DENY: &str = "deny";

pub const CLAUDE_GATE: VersionGate = VersionGate {
    agent: "claude",
    package: "Claude Code",
    min: (2, 1),
    probed: "2.1.314",
    protocol: "stream-json",
    verifiable: false,
};

#[derive(Debug, PartialEq)]
pub enum ClaudeLine {
    Init(Result<(), String>),
    Signal(ThreadSignal),
    Permission(ClaudePermission),
    Cancel(String),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaudePermission {
    pub input: Value,
    pub content: BlockedContent,
}

pub fn decode_line(line: &str) -> ClaudeLine {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return ClaudeLine::Other;
    };
    match (value["type"].as_str(), value["subtype"].as_str()) {
        (Some("system"), Some("init")) => ClaudeLine::Init(init_status(&value)),
        (Some("assistant"), _) if calls_report(&value) => {
            ClaudeLine::Signal(ThreadSignal::Reported)
        }
        (Some("result"), _) => ClaudeLine::Signal(ThreadSignal::TurnEnded(turn_end(line, &value))),
        (Some("control_request"), _) if value["request"]["subtype"] == "can_use_tool" => {
            ClaudeLine::Permission(permission(&value))
        }
        (Some("control_cancel_request"), _) => {
            ClaudeLine::Cancel(value["request_id"].as_str().unwrap_or_default().to_string())
        }
        _ => ClaudeLine::Other,
    }
}

fn permission(control: &Value) -> ClaudePermission {
    let request = &control["request"];
    let input = request["input"].clone();
    let tool = request["tool_name"].as_str().unwrap_or("tool");
    let summary = request["description"]
        .as_str()
        .or(input["command"].as_str())
        .or(input["file_path"].as_str())
        .unwrap_or(tool);
    let option = |id: &str, name: &str, kind: &str| PermissionOption {
        option_id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
    };
    ClaudePermission {
        content: BlockedContent {
            request_id: control["request_id"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            options: vec![
                option(ALLOW_ONCE, "Allow once", "allow_once"),
                option(DENY, "Deny", "reject_once"),
            ],
            pending: Vec::new(),
        },
        input,
    }
}

pub fn permission_response(request_id: &str, option_id: &str, input: &Value) -> String {
    let decision = if option_id == ALLOW_ONCE {
        json!({"behavior": "allow", "updatedInput": input})
    } else {
        json!({"behavior": "deny", "message": "The person denied this tool call."})
    };
    json!({"type": "control_response", "response": {"subtype": "success", "request_id": request_id, "response": decision}})
        .to_string()
}

fn init_status(init: &Value) -> Result<(), String> {
    let Some(servers) = init["mcp_servers"].as_array() else {
        return Ok(());
    };
    let status = servers
        .iter()
        .find(|server| server["name"] == SERVER_NAME)
        .map(|server| server["status"].as_str().unwrap_or("unknown"));
    match status {
        Some("connected") => Ok(()),
        Some(other) => Err(format!("the {SERVER_NAME} MCP server is {other}")),
        None => Err(format!("the {SERVER_NAME} MCP server is not listed")),
    }
}

fn calls_report(assistant: &Value) -> bool {
    let report = format!("mcp__{SERVER_NAME}__{REPORT_TOOL}");
    assistant["message"]["content"]
        .as_array()
        .is_some_and(|blocks| {
            blocks
                .iter()
                .any(|block| block["type"] == "tool_use" && block["name"] == report.as_str())
        })
}

fn turn_end(line: &str, result: &Value) -> TurnEnd {
    if result_is_error(line) != Some(true) {
        return TurnEnd::Succeeded;
    }
    let message = result["result"]
        .as_str()
        .or(result["subtype"].as_str())
        .unwrap_or("claude turn failed");
    TurnEnd::Failed {
        message: message.to_string(),
        transient: false,
    }
}

pub fn claude_headless_args(
    model: Option<&str>,
    mcp_config: &str,
    attachments: &Path,
    mode: PermissionMode,
) -> Vec<String> {
    let native_mode = match mode {
        PermissionMode::Ask => "default",
        PermissionMode::Yolo => "auto",
    };
    let mut args: Vec<String> = [
        "-p",
        "--verbose",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--permission-mode",
        native_mode,
        "--permission-prompt-tool",
        "stdio",
        "--mcp-config",
        mcp_config,
        "--allowedTools",
    ]
    .map(String::from)
    .into();
    args.push(own_tool_names());
    args.push("--add-dir".to_string());
    args.push(attachments.display().to_string());
    if let Some(model) = model {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    args
}

fn own_tool_names() -> String {
    let definitions = super::tools::tool_definitions();
    let names = definitions.as_array().into_iter().flatten();
    names
        .filter_map(|tool| tool["name"].as_str())
        .map(|name| format!("mcp__{SERVER_NAME}__{name}"))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn decode_claude_version(output: &str) -> Option<&str> {
    output.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{
        check_agent_version, Action, Turns, VerifiedAgents, WorkerEvent, REPORT_REMINDER,
    };
    use serde_json::json;
    use std::path::Path;

    const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s","tools":["Bash","mcp__ralph-worker__report"],"mcp_servers":[{"name":"ralph-worker","status":"connected"}]}"#;
    const REPORT: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__ralph-worker__report","input":{"status":"done","text":"ok"}}]}}"#;
    const OTHER_TOOL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#;
    const RESULT_OK: &str =
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#;
    const RESULT_ERROR: &str = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"API Error: 500"}"#;
    const ASK: &str = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"t1"}}"#;

    fn signal(line: &str) -> ThreadSignal {
        match decode_line(line) {
            ClaudeLine::Signal(signal) => signal,
            other => panic!("not a signal: {other:?}"),
        }
    }

    fn starts(actions: &[Action]) -> Vec<&str> {
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Start(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn init_needs_a_connected_report_server() {
        assert_eq!(decode_line(INIT), ClaudeLine::Init(Ok(())));
        let failed = INIT.replace("connected", "failed");
        assert!(matches!(decode_line(&failed), ClaudeLine::Init(Err(e)) if e.contains("failed")));
        let missing = INIT.replace("ralph-worker\",\"status", "other\",\"status");
        assert!(matches!(decode_line(&missing), ClaudeLine::Init(Err(_))));
        let unlisted = r#"{"type":"system","subtype":"init","session_id":"s"}"#;
        assert_eq!(decode_line(unlisted), ClaudeLine::Init(Ok(())));
    }

    #[test]
    fn only_the_report_tool_use_counts_as_reported() {
        assert_eq!(signal(REPORT), ThreadSignal::Reported);
        assert_eq!(decode_line(OTHER_TOOL), ClaudeLine::Other);
        assert_eq!(decode_line("not json"), ClaudeLine::Other);
    }

    #[test]
    fn can_use_tool_blocks_on_the_native_request_id_with_allow_and_deny() {
        let ClaudeLine::Permission(ask) = decode_line(ASK) else {
            panic!("not a permission request");
        };
        assert_eq!(ask.content.request_id, "r1");
        assert_eq!(ask.content.tool, "Bash");
        assert_eq!(ask.content.summary, "ls");
        let ids: Vec<&str> = ask
            .content
            .options
            .iter()
            .map(|option| option.option_id.as_str())
            .collect();
        assert_eq!(ids, [ALLOW_ONCE, "deny"]);
        assert_eq!(ask.input, json!({"command": "ls"}));
        let described = ASK.replace(r#""tool_use_id""#, r#""description":"list","tool_use_id""#);
        assert!(
            matches!(decode_line(&described), ClaudeLine::Permission(a) if a.content.summary == "list")
        );
        let interrupt =
            r#"{"type":"control_request","request_id":"r","request":{"subtype":"interrupt"}}"#;
        assert_eq!(decode_line(interrupt), ClaudeLine::Other);
        assert_eq!(
            decode_line(r#"{"type":"control_cancel_request","request_id":"r1"}"#),
            ClaudeLine::Cancel("r1".into())
        );
    }

    #[test]
    fn allow_echoes_the_input_without_updated_permissions_and_deny_carries_a_message() {
        let input = json!({"command": "ls"});
        let allow: Value =
            serde_json::from_str(&permission_response("r1", ALLOW_ONCE, &input)).unwrap();
        assert_eq!(
            allow,
            json!({"type": "control_response", "response": {"subtype": "success", "request_id": "r1",
                "response": {"behavior": "allow", "updatedInput": {"command": "ls"}}}})
        );
        let deny: Value = serde_json::from_str(&permission_response("r1", "deny", &input)).unwrap();
        assert_eq!(deny["response"]["request_id"], "r1");
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
        assert!(deny["response"]["response"]["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty()));
    }

    #[test]
    fn results_end_the_turn_ok_or_failed() {
        assert_eq!(
            signal(RESULT_OK),
            ThreadSignal::TurnEnded(TurnEnd::Succeeded)
        );
        assert_eq!(
            signal(RESULT_ERROR),
            ThreadSignal::TurnEnded(TurnEnd::Failed {
                message: "API Error: 500".into(),
                transient: false,
            })
        );
        let mut turns = Turns::default();
        turns.accept("task".into());
        let actions = turns.signal(signal(RESULT_ERROR), false);
        assert!(actions.contains(&Action::PostFailure("API Error: 500".into())));
    }

    #[test]
    fn an_unreported_turn_gets_one_reminder_then_is_marked() {
        let mut turns = Turns::default();
        assert_eq!(starts(&turns.accept("task".into())), ["task"]);
        let first = turns.signal(signal(RESULT_OK), false);
        assert_eq!(starts(&first), [REPORT_REMINDER]);
        let second = turns.signal(signal(RESULT_OK), false);
        assert!(starts(&second).is_empty());
        assert!(second.contains(&Action::MarkUnreported));
    }

    #[test]
    fn a_prompt_during_a_turn_waits_for_its_result() {
        let mut turns = Turns::default();
        turns.accept("task".into());
        assert!(starts(&turns.accept("go".into())).is_empty());
        assert_eq!(
            turns.signal(signal(REPORT), false),
            [Action::Emit(WorkerEvent::Reported)]
        );
        assert_eq!(starts(&turns.signal(signal(RESULT_OK), false)), ["go"]);
    }

    #[test]
    fn args_run_prompt_less_stream_json_with_the_report_server() {
        let args = claude_headless_args(Some("haiku"), "{}", Path::new("/a"), PermissionMode::Ask);
        let joined = args.join(" ");
        assert!(joined
            .starts_with("-p --verbose --input-format stream-json --output-format stream-json"));
        assert!(joined.contains("--permission-mode default --permission-prompt-tool stdio"));
        assert!(!joined.contains("--permission-prompts"));
        assert!(joined.contains(
            "--mcp-config {} --allowedTools mcp__ralph-worker__report,mcp__ralph-worker__set_status --add-dir /a --model haiku"
        ));
        let yolo = claude_headless_args(None, "{}", Path::new("/a"), PermissionMode::Yolo);
        assert!(yolo
            .join(" ")
            .contains("--permission-mode auto --permission-prompt-tool stdio"));
        assert!(!yolo.contains(&"--model".into()));
    }

    #[test]
    fn version_gate_reads_claude_version_output() {
        let verified = VerifiedAgents::default();
        assert_eq!(
            decode_claude_version("2.1.314 (Claude Code)\n"),
            Some("2.1.314")
        );
        assert_eq!(
            check_agent_version(&CLAUDE_GATE, "2.1.314", &verified),
            Ok(None)
        );
        let old = check_agent_version(&CLAUDE_GATE, "2.0.9", &verified).unwrap_err();
        assert_eq!(
            old,
            "claude 2.0.9 is too old; needs Claude Code >= 2.1 (probed 2.1.314)"
        );
        let newer = check_agent_version(&CLAUDE_GATE, "2.2.0", &verified)
            .unwrap()
            .unwrap();
        assert_eq!(
            newer,
            "claude 2.2.0 is newer than the probed 2.1.314; the stream-json protocol may differ."
        );
        assert!(check_agent_version(&CLAUDE_GATE, "garbage", &verified).is_err());
    }
}
