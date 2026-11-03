use super::acp::REPORT_TOOL;
use super::claude_headless::{calls_report, init_status};
use super::{
    decode_line, deny_line, interrupted_end, permission_response, turn_stop, CheckResult,
    ClaudeLine, Report, ThreadSignal, TurnSummary, Verdict, VersionProbe, ALLOW_ONCE,
    CLAUDE_COMMAND, CLAUDE_GATE, SERVER_NAME, VERSION_PROBE_LIMIT,
};
use crate::stream::interrupt_line;
use serde_json::Value;

pub const CLAUDE_HEADLESS_REQUIRED_CHECKS: &[&str] = &["version", "spawn", "argv"];
pub const CLAUDE_ENDED: &str = "end_turn";
pub const CLAUDE_INTERRUPTED: &str = "cancelled";
const VERIFY_INTERRUPT_ID: &str = "verify-interrupt";
pub const CLAUDE_REPORT_PROMPT: &str = "Call the mcp__ralph-worker__report tool one time with status done and text verify. Do not use other tools. Then reply ok.";
pub const CLAUDE_PERMISSION_PROMPT: &str =
    "Run the shell command `touch verify-permission.txt` and reply with ok.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeSpawn {
    Argv,
    Turns,
}

pub fn claude_verify_spawns(live: bool) -> &'static [ClaudeSpawn] {
    if live {
        &[ClaudeSpawn::Argv, ClaudeSpawn::Turns]
    } else {
        &[ClaudeSpawn::Argv]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudePlan {
    Plain,
    Allow,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeFold {
    pub summary: TurnSummary,
    pub session: Option<String>,
    pub ended: bool,
    pub mcp: Option<Result<(), String>>,
    pub reported: bool,
}

impl Default for ClaudeFold {
    fn default() -> Self {
        Self {
            summary: TurnSummary {
                stop: Err("the turn did not finish".to_string()),
                replies: 0,
                permissions: Vec::new(),
                cancel_sent: false,
            },
            session: None,
            ended: false,
            mcp: None,
            reported: false,
        }
    }
}

fn text_blocks(assistant: &Value) -> usize {
    assistant["message"]["content"]
        .as_array()
        .map_or(0, |blocks| {
            blocks
                .iter()
                .filter(|block| block["type"] == "text")
                .count()
        })
}

pub fn fold_claude_line(fold: &mut ClaudeFold, line: &str, plan: ClaudePlan) -> Vec<String> {
    let value = serde_json::from_str::<Value>(line.trim()).unwrap_or(Value::Null);
    match (value["type"].as_str(), value["subtype"].as_str()) {
        (Some("system"), Some("init")) => {
            fold.session = value["session_id"].as_str().map(str::to_string);
            fold.mcp = Some(mcp_status(&value));
            Vec::new()
        }
        (Some("assistant"), _) => {
            fold.reported |= calls_report(&value);
            fold.summary.replies += text_blocks(&value);
            if plan == ClaudePlan::Cancel && !fold.summary.cancel_sent {
                fold.summary.cancel_sent = true;
                return vec![interrupt_line(VERIFY_INTERRUPT_ID)];
            }
            Vec::new()
        }
        _ => fold_decoded(fold, line, plan),
    }
}

fn fold_decoded(fold: &mut ClaudeFold, line: &str, plan: ClaudePlan) -> Vec<String> {
    match decode_line(line) {
        ClaudeLine::Permission(ask) => {
            let reply = match plan {
                ClaudePlan::Allow => {
                    permission_response(&ask.content.request_id, ALLOW_ONCE, &ask.input)
                }
                _ => deny_line(&ask.content.request_id),
            };
            fold.summary.permissions.push(ask.content);
            vec![reply]
        }
        ClaudeLine::Signal(ThreadSignal::TurnEnded(end)) => {
            fold.ended = true;
            let end = interrupted_end(end, fold.summary.cancel_sent);
            fold.summary.stop = turn_stop(&end)
                .map(|reason| reason.to_string())
                .map_err(str::to_string);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

fn mcp_status(init: &Value) -> Result<(), String> {
    init_status(init)?;
    if init["mcp_servers"].as_array().is_none() {
        return Err("system/init listed no MCP servers".to_string());
    }
    Ok(())
}

fn unknown(name: &'static str, detail: &str) -> CheckResult {
    CheckResult {
        verdict: Verdict::Unknown,
        ..CheckResult::pass(name, detail)
    }
}

pub fn mcp_check(fold: &ClaudeFold) -> CheckResult {
    match &fold.mcp {
        Some(Ok(())) => CheckResult::pass("mcp", "ralph-worker connected"),
        Some(Err(reason)) => CheckResult::fail("mcp", reason.as_str()),
        None => unknown("mcp", "claude sent no system/init line"),
    }
}

pub fn report_check(fold: &ClaudeFold, inbox: &[Report]) -> CheckResult {
    let tool = format!("mcp__{SERVER_NAME}__{REPORT_TOOL}");
    if !matches!(fold.mcp, Some(Ok(()))) {
        return unknown("report", "skipped: ralph-worker not connected");
    }
    if let Err(error) = &fold.summary.stop {
        return CheckResult::fail("report", format!("the report turn failed: {error}"));
    }
    if fold.summary.permissions.iter().any(|ask| ask.tool == tool) {
        return CheckResult::fail("report", "claude asked permission for the report tool");
    }
    if !fold.reported {
        return CheckResult::fail("report", format!("the turn did not call {tool}"));
    }
    match inbox {
        [] => CheckResult::fail("report", "the report call wrote no inbox file"),
        [only] => CheckResult::pass("report", format!("inbox got a {} report", only.status)),
        many => CheckResult::fail("report", format!("{} reports, expected 1", many.len())),
    }
}

pub fn parse_reports(texts: &[String]) -> Vec<Report> {
    texts
        .iter()
        .filter_map(|text| serde_json::from_str(text).ok())
        .collect()
}

pub fn init_check(session: Option<&str>, expected: &str) -> CheckResult {
    match session {
        Some(found) if found == expected => {
            CheckResult::pass("init", format!("system/init carried session {found}"))
        }
        Some(found) => CheckResult::fail(
            "init",
            format!("system/init carried session {found}, expected {expected}"),
        ),
        None => CheckResult::fail("init", "claude sent no system/init line"),
    }
}

pub fn claude_probed_version(probe: &VersionProbe) -> Result<String, String> {
    let found = match probe {
        VersionProbe::Exited(output) => super::decode_claude_version(output)
            .ok_or_else(|| format!("could not read the version from `{CLAUDE_COMMAND} --version`"))?
            .to_string(),
        VersionProbe::ExitedWithError(_) => {
            return Err(format!(
                "could not run `{CLAUDE_COMMAND} --version`: it exited with an error status"
            ))
        }
        VersionProbe::Unreadable(_) => {
            return Err(format!(
                "could not read the version from `{CLAUDE_COMMAND} --version`"
            ))
        }
        VersionProbe::TimedOut => {
            return Err(format!(
                "`{CLAUDE_COMMAND} --version` did not answer within {}s",
                VERSION_PROBE_LIMIT.as_secs()
            ))
        }
        VersionProbe::Failed(error) => {
            return Err(format!(
                "could not run `{CLAUDE_COMMAND} --version`: {error}"
            ))
        }
    };
    super::check_agent_version(&CLAUDE_GATE, &found, &super::VerifiedAgents::default())?;
    Ok(found)
}

fn stderr_complaint(stderr: &str) -> Option<&str> {
    stderr.lines().find(|line| {
        let lower = line.to_lowercase();
        lower.contains("unknown option") || lower.contains("error:")
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StderrRead {
    Finished(String),
    Unfinished(String),
}

pub fn argv_check(running: bool, read: &StderrRead, exit: Result<bool, String>) -> CheckResult {
    let (StderrRead::Finished(stderr) | StderrRead::Unfinished(stderr)) = read;
    let detail = |text: &str| format!("{text}; stderr: {}", stderr.trim());
    if running
        && exit.is_ok()
        && matches!(read, StderrRead::Unfinished(_))
        && !stderr.trim().is_empty()
    {
        return CheckResult::fail(
            "argv",
            detail("the stderr read did not finish and already holds text"),
        );
    }
    match (running, stderr_complaint(stderr), exit) {
        (false, _, _) => CheckResult::fail(
            "argv",
            detail("claude exited before the settle window ended"),
        ),
        (true, Some(line), _) => {
            CheckResult::fail("argv", format!("claude complained on stderr: {line}"))
        }
        (true, None, Err(message)) => CheckResult::fail("argv", message),
        (true, None, Ok(false)) => CheckResult::fail(
            "argv",
            detail("claude exited with an error status after stdin closed"),
        ),
        (true, None, Ok(true)) => CheckResult::pass(
            "argv",
            "claude kept running with the worker argv and exited cleanly after stdin closed",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::PermissionSetup;
    use serde_json::json;

    fn feed(fold: &mut ClaudeFold, plan: ClaudePlan, lines: &[Value]) -> Vec<String> {
        lines
            .iter()
            .flat_map(|line| fold_claude_line(fold, &line.to_string(), plan))
            .collect()
    }

    fn assistant_text() -> Value {
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"ok"}]}})
    }

    fn result(is_error: bool) -> Value {
        json!({"type":"result","subtype":"success","is_error":is_error,"result":"ok"})
    }

    fn can_use_tool() -> Value {
        json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"date +%Y"}}})
    }

    #[test]
    fn stream_json_turn_folds_into_a_turn_summary() {
        let mut fold = ClaudeFold::default();
        let init = json!({"type":"system","subtype":"init","session_id":"s1","mcp_servers":[]});
        let sent = feed(
            &mut fold,
            ClaudePlan::Plain,
            &[init, assistant_text(), result(false)],
        );
        assert!(sent.is_empty());
        assert_eq!(fold.session.as_deref(), Some("s1"));
        assert!(fold.ended);
        assert_eq!(fold.summary.replies, 1);
        assert_eq!(fold.summary.stop, Ok(CLAUDE_ENDED.to_string()));
        assert!(init_check(fold.session.as_deref(), "s1").verdict == Verdict::Pass);
        assert!(init_check(fold.session.as_deref(), "s2").verdict == Verdict::Fail);
    }

    fn mcp_check_after(lines: &[Value]) -> CheckResult {
        let mut fold = ClaudeFold::default();
        feed(&mut fold, ClaudePlan::Plain, lines);
        mcp_check(&fold)
    }

    fn init_with(servers: Value) -> Value {
        json!({"type":"system","subtype":"init","session_id":"s1","mcp_servers":servers})
    }

    #[test]
    fn mcp_passes_when_ralph_worker_is_connected() {
        let check = mcp_check_after(&[init_with(
            json!([{"name":"ralph-worker","status":"connected"}]),
        )]);
        assert_eq!(check, CheckResult::pass("mcp", "ralph-worker connected"));
    }

    #[test]
    fn mcp_fails_when_ralph_worker_failed_to_connect() {
        let check = mcp_check_after(&[init_with(
            json!([{"name":"ralph-worker","status":"failed"}]),
        )]);
        assert_eq!(
            check,
            CheckResult::fail("mcp", "the ralph-worker MCP server is failed")
        );
    }

    #[test]
    fn mcp_fails_when_ralph_worker_is_not_listed() {
        let check = mcp_check_after(&[init_with(json!([{"name":"other","status":"connected"}]))]);
        assert_eq!(
            check,
            CheckResult::fail("mcp", "the ralph-worker MCP server is not listed")
        );
    }

    #[test]
    fn mcp_fails_when_init_has_no_server_array() {
        let init = json!({"type":"system","subtype":"init","session_id":"s1"});
        assert_eq!(
            mcp_check_after(&[init]),
            CheckResult::fail("mcp", "system/init listed no MCP servers")
        );
    }

    #[test]
    fn mcp_is_unknown_without_an_init_line() {
        let check = mcp_check_after(&[assistant_text()]);
        assert_eq!(check.verdict, Verdict::Unknown);
        assert_eq!(check.detail, "claude sent no system/init line");
        assert_eq!(check.name, "mcp");
    }

    fn report_tool_use() -> Value {
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"mcp__ralph-worker__report","input":{}}]}})
    }

    fn report_permission() -> Value {
        json!({"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"mcp__ralph-worker__report","input":{"status":"done"}}})
    }

    fn connected() -> Value {
        init_with(json!([{"name":"ralph-worker","status":"connected"}]))
    }

    fn inbox_report() -> Report {
        Report {
            worker: "w".to_string(),
            at: "t".to_string(),
            status: "done".to_string(),
            text: "verify".to_string(),
        }
    }

    fn report_check_after(lines: &[Value], inbox: &[Report]) -> CheckResult {
        let mut fold = ClaudeFold::default();
        feed(&mut fold, ClaudePlan::Plain, lines);
        feed(&mut fold, ClaudePlan::Plain, &[result(false)]);
        report_check(&fold, inbox)
    }

    #[test]
    fn report_is_skipped_when_ralph_worker_is_not_connected() {
        let check = report_check_after(&[report_tool_use()], &[inbox_report()]);
        assert_eq!(check.verdict, Verdict::Unknown);
        assert_eq!(check.detail, "skipped: ralph-worker not connected");
        assert_eq!(check.name, "report");
    }

    #[test]
    fn report_fails_when_the_turn_did_not_end_cleanly() {
        let mut fold = ClaudeFold::default();
        feed(
            &mut fold,
            ClaudePlan::Plain,
            &[connected(), report_tool_use()],
        );
        let check = report_check(&fold, &[inbox_report()]);
        assert_eq!(
            check,
            CheckResult::fail("report", "the report turn failed: the turn did not finish")
        );
    }

    #[test]
    fn report_fails_when_claude_asked_permission_for_the_tool() {
        let check = report_check_after(&[connected(), report_permission()], &[]);
        assert_eq!(
            check,
            CheckResult::fail("report", "claude asked permission for the report tool")
        );
    }

    #[test]
    fn report_fails_when_the_turn_never_called_the_tool() {
        let check = report_check_after(&[connected(), assistant_text()], &[]);
        assert_eq!(
            check,
            CheckResult::fail("report", "the turn did not call mcp__ralph-worker__report")
        );
    }

    #[test]
    fn report_fails_when_the_call_wrote_no_inbox_file() {
        let check = report_check_after(&[connected(), report_tool_use()], &[]);
        assert_eq!(
            check,
            CheckResult::fail("report", "the report call wrote no inbox file")
        );
    }

    #[test]
    fn report_fails_on_more_than_one_inbox_report() {
        let inbox = [inbox_report(), inbox_report()];
        let check = report_check_after(&[connected(), report_tool_use()], &inbox);
        assert_eq!(check, CheckResult::fail("report", "2 reports, expected 1"));
    }

    #[test]
    fn report_passes_on_exactly_one_inbox_report() {
        let check = report_check_after(&[connected(), report_tool_use()], &[inbox_report()]);
        assert_eq!(
            check,
            CheckResult::pass("report", "inbox got a done report")
        );
    }

    #[test]
    fn a_cancel_plan_interrupts_once_and_maps_the_failed_result() {
        let mut fold = ClaudeFold::default();
        let sent = feed(
            &mut fold,
            ClaudePlan::Cancel,
            &[assistant_text(), assistant_text(), result(true)],
        );
        assert_eq!(sent.len(), 1);
        assert!(sent[0].contains("interrupt"));
        assert_eq!(fold.summary.stop, Ok(CLAUDE_INTERRUPTED.to_string()));
    }

    #[test]
    fn can_use_tool_folds_into_an_allow_once_permission() {
        let mut fold = ClaudeFold::default();
        let sent = feed(
            &mut fold,
            ClaudePlan::Allow,
            &[can_use_tool(), assistant_text(), result(false)],
        );
        let reply: Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "allow");
        assert_eq!(reply["response"]["request_id"], "r1");
        let check = crate::worker::permission_check(
            &fold.summary,
            CLAUDE_ENDED,
            &PermissionSetup::claude_headless(),
        );
        assert_eq!(check.verdict, Verdict::Pass, "{}", check.detail);
    }

    #[test]
    fn a_plain_plan_denies_a_permission_request() {
        let mut fold = ClaudeFold::default();
        let sent = feed(&mut fold, ClaudePlan::Plain, &[can_use_tool()]);
        let reply: Value = serde_json::from_str(&sent[0]).unwrap();
        assert_eq!(reply["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn argv_fails_on_an_early_exit_or_a_stderr_complaint() {
        let finished = |text: &str| StderrRead::Finished(text.to_string());
        assert_eq!(
            argv_check(false, &finished(""), Ok(true)).verdict,
            Verdict::Fail
        );
        let unknown = argv_check(true, &finished("error: unknown option '--x'\n"), Ok(true));
        assert_eq!(unknown.verdict, Verdict::Fail);
        assert_eq!(
            argv_check(true, &finished(""), Ok(true)).verdict,
            Verdict::Pass
        );
    }

    #[test]
    fn an_unfinished_empty_stderr_read_with_a_clean_exit_passes_argv() {
        let check = argv_check(true, &StderrRead::Unfinished(String::new()), Ok(true));
        assert_eq!(check.verdict, Verdict::Pass, "{}", check.detail);
    }

    #[test]
    fn an_unfinished_stderr_read_with_text_does_not_pass_the_argv_check() {
        let check = argv_check(true, &StderrRead::Unfinished("warming up".into()), Ok(true));
        assert_eq!(check.verdict, Verdict::Fail, "{}", check.detail);
        assert!(check.detail.contains("warming up"), "{}", check.detail);
    }

    #[test]
    fn a_missing_exit_is_reported_even_when_the_stderr_read_is_unfinished() {
        let read = StderrRead::Unfinished("warming up".into());
        let check = argv_check(true, &read, Err("claude did not exit within 10s".into()));
        assert_eq!(check.verdict, Verdict::Fail, "{}", check.detail);
        assert!(check.detail.contains("did not exit"), "{}", check.detail);
    }

    #[test]
    fn the_report_prompt_names_the_tool_status_and_text() {
        assert!(CLAUDE_REPORT_PROMPT.contains("mcp__ralph-worker__report"));
        assert!(CLAUDE_REPORT_PROMPT.contains("status done and text verify"));
    }

    #[test]
    fn parse_reports_keeps_valid_reports_and_skips_the_rest() {
        let valid = r#"{"worker":"verify","at":"t","status":"done","text":"verify"}"#;
        let found = parse_reports(&[valid.to_string(), "not json".to_string(), "{}".to_string()]);
        assert_eq!(
            found,
            vec![Report {
                worker: "verify".into(),
                at: "t".into(),
                status: "done".into(),
                text: "verify".into(),
            }]
        );
        assert!(parse_reports(&[]).is_empty());
    }

    #[test]
    fn the_permission_prompt_writes_a_file() {
        assert!(CLAUDE_PERMISSION_PROMPT.contains("touch verify-permission.txt"));
        assert!(!CLAUDE_PERMISSION_PROMPT.contains("date"));
    }

    #[test]
    fn the_argv_check_runs_in_its_own_spawn_before_live_turns() {
        assert_eq!(claude_verify_spawns(false), &[ClaudeSpawn::Argv]);
        assert_eq!(
            claude_verify_spawns(true),
            &[ClaudeSpawn::Argv, ClaudeSpawn::Turns]
        );
    }
}
