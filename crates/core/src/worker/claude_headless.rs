use super::acp::REPORT_TOOL;
use super::{ThreadSignal, TurnEnd, VersionGate, SERVER_NAME};
use crate::stream::result_is_error;
use serde_json::Value;
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
    Other,
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
        _ => ClaudeLine::Other,
    }
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
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--verbose",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--permission-mode",
        "auto",
        "--permission-prompts",
        "none",
        "--mcp-config",
        mcp_config,
        "--add-dir",
    ]
    .map(String::from)
    .into();
    args.push(attachments.display().to_string());
    if let Some(model) = model {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    args
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
    use std::path::Path;

    const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s","tools":["Bash","mcp__ralph-worker__report"],"mcp_servers":[{"name":"ralph-worker","status":"connected"}]}"#;
    const REPORT: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"mcp__ralph-worker__report","input":{"status":"done","text":"ok"}}]}}"#;
    const OTHER_TOOL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#;
    const RESULT_OK: &str =
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done"}"#;
    const RESULT_ERROR: &str = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"API Error: 500"}"#;

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
        assert_eq!(
            decode_line(r#"{"type":"control_request","request_id":"r"}"#),
            ClaudeLine::Other
        );
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
        let args = claude_headless_args(Some("haiku"), "{}", Path::new("/a"));
        let joined = args.join(" ");
        assert!(joined
            .starts_with("-p --verbose --input-format stream-json --output-format stream-json"));
        assert!(joined.contains("--permission-mode auto --permission-prompts none"));
        assert!(joined.contains("--mcp-config {} --add-dir /a --model haiku"));
        assert!(!claude_headless_args(None, "{}", Path::new("/a")).contains(&"--model".into()));
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
