use super::acp::REPORT_TOOL;
use super::{
    allowed_tools, Action, ActivityMark, BlockedContent, PermissionMode, PermissionOption, Report,
    ReportStatus, ThreadSignal, TurnEnd, Turns, VersionGate, SERVER_NAME,
};
use crate::stream::{interrupt_line, result_is_error};
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
    verifiable: true,
};

#[derive(Debug, PartialEq)]
pub enum ClaudeLine {
    Init(Result<String, String>),
    Signal(ThreadSignal),
    Permission(ClaudePermission),
    Cancel(String),
    Malformed,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaudePermission {
    pub input: Value,
    pub content: BlockedContent,
}

pub fn stuck_permission_report(id: &str, at: &str) -> Report {
    Report {
        worker: id.to_string(),
        at: at.to_string(),
        status: ReportStatus::Blocked.as_str().to_string(),
        text: "claude sent a permission request ralph can't answer (no request_id or no input object); the worker is stuck — close it.".to_string(),
    }
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
            permission(&value).map_or(ClaudeLine::Malformed, ClaudeLine::Permission)
        }
        (Some("control_cancel_request"), _) => control_cancel_request(&value),
        _ => ClaudeLine::Other,
    }
}

fn control_cancel_request(control: &Value) -> ClaudeLine {
    request_id(control).map_or(ClaudeLine::Other, |id| ClaudeLine::Cancel(id.to_string()))
}

fn request_id(control: &Value) -> Option<&str> {
    control["request_id"].as_str().filter(|id| !id.is_empty())
}

fn permission(control: &Value) -> Option<ClaudePermission> {
    let request = &control["request"];
    let request_id = request_id(control)?;
    let input = request["input"].clone();
    if !input.is_object() {
        return None;
    }
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
    Some(ClaudePermission {
        content: BlockedContent {
            request_id: request_id.to_string(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            options: vec![
                option(ALLOW_ONCE, "Allow once", "allow_once"),
                option(DENY, "Deny", "reject_once"),
            ],
            pending: Vec::new(),
        },
        input,
    })
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

pub fn deny_line(request_id: &str) -> String {
    permission_response(request_id, DENY, &Value::Null)
}

pub fn interrupt_lines(pending: &[String], turn_id: &str) -> Vec<String> {
    pending
        .iter()
        .map(|request_id| deny_line(request_id))
        .chain([interrupt_line(&format!("interrupt-{turn_id}"))])
        .collect()
}

pub fn auto_answer(mode: PermissionMode, ask: &ClaudePermission) -> Option<String> {
    match mode {
        PermissionMode::Yolo => Some(permission_response(
            &ask.content.request_id,
            ALLOW_ONCE,
            &ask.input,
        )),
        PermissionMode::Ask => None,
    }
}

pub(super) fn init_status(init: &Value) -> Result<String, String> {
    let session = init["session_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("claude init has no session id")?;
    let Some(servers) = init["mcp_servers"].as_array() else {
        return Ok(session.to_string());
    };
    let status = servers
        .iter()
        .find(|server| server["name"] == SERVER_NAME)
        .map(|server| server["status"].as_str().unwrap_or("unknown"));
    match status {
        Some("connected") => Ok(session.to_string()),
        Some(other) => Err(format!("the {SERVER_NAME} MCP server is {other}")),
        None => Err(format!("the {SERVER_NAME} MCP server is not listed")),
    }
}

pub(super) fn calls_report(assistant: &Value) -> bool {
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

pub fn interrupted_end(end: TurnEnd, interrupt_sent: bool) -> TurnEnd {
    match end {
        TurnEnd::Succeeded => TurnEnd::Succeeded,
        _ if interrupt_sent => TurnEnd::Interrupted,
        other => other,
    }
}

pub fn bind_started_turn(turns: &mut Turns, mut actions: Vec<Action>, seq: u64) -> Vec<Action> {
    if actions
        .iter()
        .any(|action| matches!(action, Action::Start(_)))
    {
        actions.extend(turns.bind_turn(Some(format!("turn-{seq}"))));
    }
    actions
}

#[derive(Clone, Copy)]
pub struct ClaudeHeadlessArgs<'a> {
    pub model: Option<&'a str>,
    pub mcp_config: &'a str,
    pub attachments: &'a Path,
    pub mode: PermissionMode,
    pub session: ClaudeSession<'a>,
    pub copied: &'a [String],
}

pub fn claude_headless_args(options: &ClaudeHeadlessArgs) -> Vec<String> {
    let ClaudeHeadlessArgs {
        model,
        mcp_config,
        attachments,
        mode,
        session,
        copied,
    } = *options;
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
        "--strict-mcp-config",
        "--allowedTools",
    ]
    .map(String::from)
    .into();
    args.push(allowed_tools(copied));
    args.push("--add-dir".to_string());
    args.push(attachments.display().to_string());
    if let Some(model) = model {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    let (flag, id) = match session {
        ClaudeSession::New(id) => ("--session-id", id),
        ClaudeSession::Resume(id) => ("--resume", id),
    };
    args.extend([flag.to_string(), id.to_string()]);
    args
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClaudeSession<'a> {
    New(&'a str),
    Resume(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartStep {
    WriteOwner,
    WriteRecord,
}

pub fn start_steps(session: &ClaudeSession) -> &'static [StartStep] {
    match session {
        ClaudeSession::New(_) => &[StartStep::WriteOwner, StartStep::WriteRecord],
        ClaudeSession::Resume(_) => &[StartStep::WriteOwner],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownStep {
    PostLostTurn,
    ClearActivity,
    Stop,
    Forget,
}

pub fn shutdown_steps(running: bool) -> &'static [ShutdownStep] {
    if running {
        &[
            ShutdownStep::PostLostTurn,
            ShutdownStep::ClearActivity,
            ShutdownStep::Stop,
            ShutdownStep::Forget,
        ]
    } else {
        &[
            ShutdownStep::ClearActivity,
            ShutdownStep::Stop,
            ShutdownStep::Forget,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFault {
    OwnerWrite,
    RecordWrite,
    SpawnFailed,
    InitFailed,
    NotAdopted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leftover {
    StopChild,
    RemoveOwner,
    RemoveRecord,
    RemoveConfig,
}

pub fn cleanup(fault: StartFault) -> &'static [Leftover] {
    match fault {
        StartFault::SpawnFailed => &[Leftover::RemoveConfig],
        StartFault::OwnerWrite => &[Leftover::StopChild, Leftover::RemoveConfig],
        StartFault::RecordWrite => &[
            Leftover::StopChild,
            Leftover::RemoveOwner,
            Leftover::RemoveConfig,
        ],
        StartFault::InitFailed | StartFault::NotAdopted => &[
            Leftover::StopChild,
            Leftover::RemoveOwner,
            Leftover::RemoveRecord,
            Leftover::RemoveConfig,
        ],
    }
}

pub fn uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes.map(|byte| format!("{byte:02x}")).concat();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

pub fn resumed_session(expected: Option<&str>, found: String) -> Result<String, String> {
    match expected {
        Some(expected) if expected != found => Err(format!(
            "claude resumed session {found} instead of {expected}"
        )),
        _ => Ok(found),
    }
}

pub fn turn_lost_at_resume(activity: Option<&str>) -> bool {
    activity
        .and_then(ActivityMark::parse)
        .is_some_and(|mark| mark.busy())
}

pub fn decode_claude_version(output: &str) -> Option<&str> {
    output.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{
        check_agent_version, Action, InterruptOutcome, Turns, VerifiedAgents, WorkerEvent,
        REPORT_REMINDER,
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
        assert_eq!(decode_line(INIT), ClaudeLine::Init(Ok("s".into())));
        let failed = INIT.replace("connected", "failed");
        assert!(matches!(decode_line(&failed), ClaudeLine::Init(Err(e)) if e.contains("failed")));
        let missing = INIT.replace("ralph-worker\",\"status", "other\",\"status");
        assert!(matches!(decode_line(&missing), ClaudeLine::Init(Err(_))));
        let unlisted = r#"{"type":"system","subtype":"init","session_id":"s"}"#;
        assert_eq!(decode_line(unlisted), ClaudeLine::Init(Ok("s".into())));
        let anonymous = r#"{"type":"system","subtype":"init"}"#;
        assert!(
            matches!(decode_line(anonymous), ClaudeLine::Init(Err(e)) if e.contains("session id"))
        );
    }

    #[test]
    fn a_resumed_worker_must_keep_its_session_id() {
        assert_eq!(resumed_session(None, "n".into()), Ok("n".into()));
        assert_eq!(resumed_session(Some("s"), "s".into()), Ok("s".into()));
        assert_eq!(
            resumed_session(Some("s"), "n".into()),
            Err("claude resumed session n instead of s".into())
        );
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
    fn permission_lines_without_a_request_id_or_input_are_malformed() {
        let malformed = |line: &str| matches!(decode_line(line), ClaudeLine::Malformed);
        assert!(malformed(&ASK.replace(r#""request_id":"r1","#, "")));
        assert!(malformed(
            &ASK.replace(r#""request_id":"r1""#, r#""request_id":"""#)
        ));
        assert!(malformed(&ASK.replace(r#""input":{"command":"ls"},"#, "")));
        assert!(malformed(&ASK.replace(r#"{"command":"ls"}"#, r#""ls""#)));
        let empty_input = ASK.replace(r#"{"command":"ls"}"#, "{}");
        assert!(matches!(
            decode_line(&empty_input),
            ClaudeLine::Permission(ask) if ask.input == json!({})
        ));
    }

    #[test]
    fn a_cancel_without_a_request_id_is_ignored() {
        assert_eq!(
            decode_line(r#"{"type":"control_cancel_request"}"#),
            ClaudeLine::Other
        );
        assert_eq!(
            decode_line(r#"{"type":"control_cancel_request","request_id":""}"#),
            ClaudeLine::Other
        );
    }

    #[test]
    fn yolo_answers_allow_once_and_ask_answers_nothing() {
        let ClaudeLine::Permission(ask) = decode_line(ASK) else {
            panic!("not a permission request");
        };
        let line = auto_answer(PermissionMode::Yolo, &ask).unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(answer["response"]["request_id"], "r1");
        assert_eq!(answer["response"]["response"]["behavior"], "allow");
        assert_eq!(
            answer["response"]["response"]["updatedInput"],
            json!({"command": "ls"})
        );
        assert_eq!(auto_answer(PermissionMode::Ask, &ask), None);
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

    fn failed() -> TurnEnd {
        TurnEnd::Failed {
            message: "x".into(),
            transient: false,
        }
    }

    #[test]
    fn a_failed_end_after_an_interrupt_counts_as_interrupted() {
        assert_eq!(interrupted_end(failed(), true), TurnEnd::Interrupted);
        assert_eq!(interrupted_end(failed(), false), failed());
        assert_eq!(
            interrupted_end(TurnEnd::Succeeded, true),
            TurnEnd::Succeeded
        );
    }

    fn started(turns: &mut Turns, seq: u64) -> Vec<Action> {
        let actions = turns.accept("task".into());
        bind_started_turn(turns, actions, seq)
    }

    #[test]
    fn an_interrupt_while_running_fires_once_the_turn_is_bound_and_cancels_on_the_error_result() {
        let mut turns = Turns::default();
        let actions = started(&mut turns, 1);
        assert_eq!(starts(&actions), ["task"]);
        assert!(!actions.iter().any(|a| matches!(a, Action::Interrupt(_))));
        let sent = turns.interrupt("stop".into(), "e1".into());
        assert_eq!(sent, [Action::Interrupt("turn-1".into())]);
        let end = interrupted_end(failed(), turns.interrupt_sent());
        let done = turns.signal(ThreadSignal::TurnEnded(end), false);
        assert!(done.contains(&Action::InterruptResult {
            entry: "e1".into(),
            outcome: InterruptOutcome::Cancelled
        }));
    }

    #[test]
    fn a_process_that_exits_after_the_interrupt_resolves_it_as_gone() {
        let mut turns = Turns::default();
        started(&mut turns, 1);
        let sent = turns.interrupt("stop".into(), "e1".into());
        assert_eq!(sent, [Action::Interrupt("turn-1".into())]);
        assert_eq!(
            turns.process_gone(),
            [Action::InterruptResult {
                entry: "e1".into(),
                outcome: InterruptOutcome::Gone
            }]
        );
    }

    #[test]
    fn an_interrupt_while_idle_reports_not_cancelled_and_starts_the_prompt() {
        let mut turns = Turns::default();
        let actions = turns.interrupt("stop".into(), "e1".into());
        let actions = bind_started_turn(&mut turns, actions, 1);
        assert_eq!(
            actions[0],
            Action::InterruptResult {
                entry: "e1".into(),
                outcome: InterruptOutcome::Sent
            }
        );
        assert_eq!(starts(&actions), ["stop"]);
        assert!(!actions.iter().any(|a| matches!(a, Action::Interrupt(_))));
    }

    #[test]
    fn binding_runs_on_the_same_turns_borrow_and_a_queued_interrupt_fires_at_start() {
        let turns = std::sync::Mutex::new(Turns::default());
        let mut guard = turns.lock().unwrap();
        let actions = guard.accept("task".into());
        let actions = bind_started_turn(&mut guard, actions, 7);
        assert_eq!(starts(&actions), ["task"]);
        assert_eq!(
            guard.interrupt("s".into(), "e".into()),
            [Action::Interrupt("turn-7".into())]
        );
    }

    #[test]
    fn args_run_prompt_less_stream_json_with_the_report_server() {
        let args = claude_headless_args(&ClaudeHeadlessArgs {
            model: Some("haiku"),
            mcp_config: "{}",
            attachments: Path::new("/a"),
            mode: PermissionMode::Ask,
            session: ClaudeSession::New("sid"),
            copied: &[],
        });
        let joined = args.join(" ");
        assert!(joined
            .starts_with("-p --verbose --input-format stream-json --output-format stream-json"));
        assert!(joined.contains("--permission-mode default --permission-prompt-tool stdio"));
        assert!(!joined.contains("--permission-prompts"));
        assert!(joined.contains(
            "--mcp-config {} --strict-mcp-config --allowedTools mcp__ralph-worker__report,mcp__ralph-worker__set_status --add-dir /a --model haiku"
        ));
        assert!(!args.contains(&"--resume".into()));
        assert!(args.contains(&"--strict-mcp-config".into()));
        let yolo = claude_headless_args(&ClaudeHeadlessArgs {
            model: None,
            mcp_config: "{}",
            attachments: Path::new("/a"),
            mode: PermissionMode::Yolo,
            session: ClaudeSession::New("sid"),
            copied: &[],
        });
        assert!(yolo
            .join(" ")
            .contains("--permission-mode auto --permission-prompt-tool stdio"));
        assert!(!yolo.contains(&"--model".into()));
        let resumed = claude_headless_args(&ClaudeHeadlessArgs {
            model: None,
            mcp_config: "{}",
            attachments: Path::new("/a"),
            mode: PermissionMode::Ask,
            session: ClaudeSession::Resume("abc"),
            copied: &[],
        });
        assert!(resumed.join(" ").ends_with("--resume abc"));
        assert!(!resumed.contains(&"--session-id".into()));
        assert!(resumed.contains(&"--strict-mcp-config".into()));
    }

    #[test]
    fn args_allow_the_copied_servers_and_pass_the_config_path() {
        let copied = ["mcptools".to_string(), "docs".to_string()];
        let args = claude_headless_args(&ClaudeHeadlessArgs {
            model: None,
            mcp_config: "/s/workers/A.json",
            attachments: Path::new("/a"),
            mode: PermissionMode::Ask,
            session: ClaudeSession::New("sid"),
            copied: &copied,
        });
        assert!(args.join(" ").contains(
            "--mcp-config /s/workers/A.json --strict-mcp-config --allowedTools mcp__ralph-worker__report,mcp__ralph-worker__set_status,mcp__mcptools__*,mcp__docs__* --add-dir /a"
        ));
    }

    #[test]
    fn a_new_session_passes_its_id_and_never_resume() {
        let args = claude_headless_args(&ClaudeHeadlessArgs {
            model: None,
            mcp_config: "{}",
            attachments: Path::new("/a"),
            mode: PermissionMode::Ask,
            session: ClaudeSession::New("11111111-2222-4333-8444-555555555555"),
            copied: &[],
        });
        assert!(args
            .join(" ")
            .ends_with("--session-id 11111111-2222-4333-8444-555555555555"));
        assert!(!args.contains(&"--resume".into()));
    }

    #[test]
    fn shutdown_steps_clear_the_activity_marker_right_after_the_report_and_before_the_stop() {
        assert_eq!(
            shutdown_steps(true),
            [
                ShutdownStep::PostLostTurn,
                ShutdownStep::ClearActivity,
                ShutdownStep::Stop,
                ShutdownStep::Forget
            ]
        );
        assert_eq!(
            shutdown_steps(false),
            [
                ShutdownStep::ClearActivity,
                ShutdownStep::Stop,
                ShutdownStep::Forget
            ]
        );
    }

    #[test]
    fn a_new_session_writes_the_owner_then_the_record_and_a_resume_only_the_owner() {
        assert_eq!(
            start_steps(&ClaudeSession::New("a")),
            [StartStep::WriteOwner, StartStep::WriteRecord]
        );
        assert_eq!(
            start_steps(&ClaudeSession::Resume("a")),
            [StartStep::WriteOwner]
        );
    }

    #[test]
    fn each_start_fault_cleans_exactly_what_it_left_behind() {
        assert_eq!(cleanup(StartFault::SpawnFailed), [Leftover::RemoveConfig]);
        assert_eq!(
            cleanup(StartFault::OwnerWrite),
            [Leftover::StopChild, Leftover::RemoveConfig]
        );
        assert_eq!(
            cleanup(StartFault::RecordWrite),
            [
                Leftover::StopChild,
                Leftover::RemoveOwner,
                Leftover::RemoveConfig
            ]
        );
        let full = [
            Leftover::StopChild,
            Leftover::RemoveOwner,
            Leftover::RemoveRecord,
            Leftover::RemoveConfig,
        ];
        assert_eq!(cleanup(StartFault::InitFailed), full);
        assert_eq!(cleanup(StartFault::NotAdopted), full);
    }

    #[test]
    fn uuid_v4_formats_lowercase_hex_with_version_and_variant() {
        let id = uuid_v4([
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0xff, 0x77, 0xff, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ]);
        assert_eq!(id, "00112233-4455-4f77-bf99-aabbccddeeff");
        let parts = id.split('-').map(str::len).collect::<Vec<_>>();
        assert_eq!(parts, [8, 4, 4, 4, 12]);
        assert_eq!(uuid_v4([0; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_v4([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
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
            "claude 2.2.0 is newer than the probed 2.1.314; the stream-json protocol may differ; run `ralph worker verify claude` to check this version."
        );
        assert!(check_agent_version(&CLAUDE_GATE, "garbage", &verified).is_err());
    }

    #[test]
    fn claude_headless_warning_is_silent_for_a_recorded_version() {
        let recorded = VerifiedAgents::default().record(crate::worker::verified_entry(
            "claude",
            "2.2.0",
            false,
            "2026-11-02",
        ));
        assert_eq!(
            check_agent_version(&CLAUDE_GATE, "2.2.0", &recorded),
            Ok(None)
        );
        let unrecorded = check_agent_version(&CLAUDE_GATE, "2.2.1", &recorded)
            .unwrap()
            .unwrap();
        assert!(
            unrecorded.ends_with("run `ralph worker verify claude` to check this version."),
            "{unrecorded}"
        );
    }

    #[test]
    fn a_stuck_permission_report_blocks_the_worker_with_the_close_hint() {
        let report = stuck_permission_report("A", "2026-01-01T00:00:00Z");
        assert_eq!(report.worker, "A");
        assert_eq!(report.status, "blocked");
        assert_eq!(
            report.text,
            "claude sent a permission request ralph can't answer (no request_id or no input object); the worker is stuck — close it."
        );
    }

    #[test]
    fn interrupt_lines_deny_each_pending_request_before_the_interrupt() {
        let pending = ["r1".to_string(), "r2".to_string()];
        let lines = interrupt_lines(&pending, "turn-3");
        assert_eq!(
            lines,
            [
                permission_response("r1", "deny", &Value::Null),
                permission_response("r2", "deny", &Value::Null),
                interrupt_line("interrupt-turn-3"),
            ]
        );
    }

    #[test]
    fn deny_line_answers_the_request_with_a_deny() {
        assert_eq!(
            deny_line("r9"),
            permission_response("r9", "deny", &Value::Null)
        );
        assert!(deny_line("r9").contains(r#""request_id":"r9""#));
        assert!(deny_line("r9").contains(r#""behavior":"deny""#));
    }

    #[test]
    fn interrupt_lines_without_pending_requests_is_only_the_interrupt() {
        assert_eq!(
            interrupt_lines(&[], "turn-3"),
            [interrupt_line("interrupt-turn-3")]
        );
    }

    #[test]
    fn a_running_mark_left_at_resume_means_the_turn_was_lost() {
        assert!(turn_lost_at_resume(Some("42 running")));
        assert!(turn_lost_at_resume(Some("42 running\n")));
        assert!(!turn_lost_at_resume(Some("42 idle")));
        assert!(!turn_lost_at_resume(Some("running")));
        assert!(!turn_lost_at_resume(Some("")));
        assert!(!turn_lost_at_resume(None));
    }
}
