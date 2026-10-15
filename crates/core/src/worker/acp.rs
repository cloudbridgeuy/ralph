use super::{Report, ReportStatus, SERVER_NAME, WORKER_PROMPT};
use crate::chat_memory::Trigger;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const ACP_PROTOCOL_VERSION: u16 = 1;
pub const ACP_AGENT_VERSION: &str = "2026.10.14";
pub const CURSOR_HARNESS: &str = "cursor";
pub const CURSOR_SPAWN_DIR: &str = "cursor-spawn";
pub const CURSOR_CLOSE_DIR: &str = "cursor-close";
pub const CURSOR_ERROR_DIR: &str = "cursor-error";
pub const CURSOR_OWNER_DIR: &str = "cursor";
pub const CURSOR_ANSWER_DIR: &str = "cursor-answer";
pub const CURSOR_PROMPT_DIR: &str = "cursor-prompt";
pub const CURSOR_SESSION_DIR: &str = "cursor-session";
pub const HANDOFF_TIMEOUT: Duration = Duration::from_secs(30);
pub const HANDOFF_POLL: Duration = Duration::from_millis(250);
const AUTH_REQUIRED_CODE: i32 = -32000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorFiles {
    pub spawn: PathBuf,
    pub close: PathBuf,
    pub error: PathBuf,
    pub owner: PathBuf,
    pub answer: PathBuf,
    pub prompt: PathBuf,
    pub session: PathBuf,
}

impl CursorFiles {
    pub fn new(store: &Path, id: &str) -> Self {
        Self {
            spawn: store.join(CURSOR_SPAWN_DIR).join(id),
            close: store.join(CURSOR_CLOSE_DIR).join(id),
            error: store.join(CURSOR_ERROR_DIR).join(id),
            owner: store.join(CURSOR_OWNER_DIR).join(id),
            answer: store.join(CURSOR_ANSWER_DIR).join(id),
            prompt: store.join(CURSOR_PROMPT_DIR).join(id),
            session: store.join(CURSOR_SESSION_DIR).join(id),
        }
    }

    pub fn all(&self) -> [&Path; 6] {
        [
            &self.spawn,
            &self.close,
            &self.error,
            &self.owner,
            &self.answer,
            &self.prompt,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnRequest {
    pub worktree: PathBuf,
    pub exe: PathBuf,
    pub inbox: PathBuf,
    pub task: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerRecord {
    pub chat_pid: u32,
    pub chat_start: String,
    pub group: u32,
    pub group_start: String,
}

impl OwnerRecord {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub cwd: PathBuf,
    pub inbox: PathBuf,
    #[serde(default)]
    pub store: Option<PathBuf>,
}

impl SessionRecord {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    Go,
    Skip(String),
}

pub fn resume_decision(
    id: &str,
    record: &SessionRecord,
    store: &Path,
    loads_session: bool,
) -> Resume {
    let skip = |why: String| Resume::Skip(format!("cursor session {id} not resumed: {why}"));
    match record.store.as_deref() {
        None => skip("the record has no owner".to_string()),
        Some(owner) if owner != store => skip(format!(
            "it belongs to the chat store {}, this chat uses {}",
            owner.display(),
            store.display()
        )),
        Some(_) if !loads_session => skip("the agent does not support session/load".to_string()),
        Some(_) => Resume::Go,
    }
}

pub fn prompt_entry_name(nanos: u64, pid: u32) -> String {
    format!("{nanos:020}-{pid}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerFacts {
    pub chat_alive: bool,
    pub group_alive: bool,
    pub mine: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    Reclaim { kill_group: bool },
    Wait,
    Busy,
}

pub fn sweep_decision(facts: OwnerFacts) -> Sweep {
    match (facts.chat_alive, facts.mine) {
        (true, true) => Sweep::Wait,
        (true, false) => Sweep::Busy,
        (false, _) => Sweep::Reclaim {
            kill_group: facts.group_alive,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedContent {
    pub tool: String,
    pub summary: String,
    pub options: Vec<PermissionOption>,
}

impl BlockedContent {
    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

pub fn permission_content(request: &Value) -> BlockedContent {
    let call = &request["toolCall"];
    let text = |key: &str| call[key].as_str().filter(|text| !text.trim().is_empty());
    let tool = text("name").or_else(|| text("kind")).unwrap_or("tool");
    let summary = text("title").unwrap_or("permission requested");
    let options = request
        .get("options")
        .and_then(|options| serde_json::from_value(options.clone()).ok())
        .unwrap_or_default();
    BlockedContent {
        tool: tool.to_string(),
        summary: summary.to_string(),
        options,
    }
}

pub fn match_option<'a>(
    options: &'a [PermissionOption],
    text: &str,
) -> Option<&'a PermissionOption> {
    let text = text.trim();
    options.iter().find(|option| option.option_id == text)
}

const REPORT_TOOL: &str = "report";

fn report_tool_label() -> String {
    format!("{SERVER_NAME}-{REPORT_TOOL}: {REPORT_TOOL}")
}

/// The allow-once option to answer with, only for the ralph worker `report` tool.
/// Every other request returns `None` and waits for the person.
pub fn auto_allow_option(content: &BlockedContent) -> Option<&PermissionOption> {
    let label = report_tool_label();
    let is_report = content.tool.trim() == label || content.summary.trim() == label;
    is_report
        .then(|| {
            content
                .options
                .iter()
                .find(|option| option.kind == "allow_once")
        })
        .flatten()
}

/// A close may proceed when the chat that owns the worker is the caller itself,
/// which is the chat or one of the processes below it (claude, chat-mcp).
pub fn caller_owns_worker(owner_chat_pid: u32, caller_ancestry: &[u32]) -> bool {
    caller_ancestry.contains(&owner_chat_pid)
}

pub fn no_option_message(options: &[PermissionOption]) -> String {
    let ids: Vec<&str> = options
        .iter()
        .map(|option| option.option_id.as_str())
        .collect();
    format!(
        "No permission option matches that answer. Valid optionIds: {}.",
        ids.join(", ")
    )
}

pub fn permission_reply_allowed(trigger: Option<Trigger>) -> Result<(), String> {
    match trigger {
        Some(Trigger::User) => Ok(()),
        _ => Err(
            "Only the person can answer a cursor permission request. Relay it to them and wait for their answer."
                .to_string(),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    Pending,
    Done,
    Failed(String),
    TimedOut,
}

pub fn spawn_handoff(ready: bool, error: Option<&str>, elapsed: Duration) -> Handoff {
    match (error, ready) {
        (Some(error), _) => Handoff::Failed(error.trim().to_string()),
        (None, true) => Handoff::Done,
        (None, false) if elapsed >= HANDOFF_TIMEOUT => Handoff::TimedOut,
        (None, false) => Handoff::Pending,
    }
}

pub fn close_handoff(owner_exists: bool, elapsed: Duration) -> Handoff {
    if !owner_exists {
        Handoff::Done
    } else if elapsed >= HANDOFF_TIMEOUT {
        Handoff::TimedOut
    } else {
        Handoff::Pending
    }
}

pub fn request_stale(requested_at: u64, now: u64) -> bool {
    now.saturating_sub(requested_at) >= HANDOFF_TIMEOUT.as_secs()
}

pub fn parse_agent_version(output: &str) -> Option<&str> {
    let token = output.split_whitespace().next()?;
    let version = token.split('-').next()?;
    version.chars().next()?.is_ascii_digit().then_some(version)
}

pub fn version_warning(output: &str) -> Option<String> {
    match parse_agent_version(output) {
        Some(ACP_AGENT_VERSION) => None,
        Some(other) => Some(format!(
            "cursor agent {other} differs from the tested {ACP_AGENT_VERSION}; the cursor worker may misbehave."
        )),
        None => Some(format!(
            "could not read the cursor agent version from '{}'; tested with {ACP_AGENT_VERSION}.",
            output.trim()
        )),
    }
}

pub fn check_protocol(version: u16) -> Result<(), String> {
    if version == ACP_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(format!(
            "cursor agent speaks ACP protocol {version}; ralph supports {ACP_PROTOCOL_VERSION}."
        ))
    }
}

pub fn session_error(code: i32, message: &str) -> String {
    if code == AUTH_REQUIRED_CODE {
        "cursor agent is not logged in; run `agent login` and spawn the worker again.".to_string()
    } else {
        format!("cursor agent could not start a session: {message}")
    }
}

pub fn task_prompt(task: &str) -> String {
    format!("{WORKER_PROMPT}\n\n{task}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Reported,
    Remind,
    Unreported,
    Problem(String),
    Closed,
}

pub fn turn_outcome(
    stop: Result<&str, &str>,
    reported: bool,
    reminded: bool,
    closing: bool,
) -> TurnOutcome {
    if closing {
        return TurnOutcome::Closed;
    }
    if reported {
        return TurnOutcome::Reported;
    }
    match stop {
        Err(message) => TurnOutcome::Problem(message.to_string()),
        Ok("end_turn") if reminded => TurnOutcome::Unreported,
        Ok("end_turn") => TurnOutcome::Remind,
        Ok(other) => {
            TurnOutcome::Problem(format!("the cursor agent ended its turn with `{other}`"))
        }
    }
}

pub fn is_report_tool_call(update: &Value) -> bool {
    let title = format!("{SERVER_NAME}: {REPORT_TOOL}");
    update["sessionUpdate"] == "tool_call_update"
        && update["title"].as_str().is_some_and(|v| v.trim() == title)
}

pub fn prompt_failure_report(id: &str, at: &str, problem: &str) -> Report {
    Report {
        worker: id.to_string(),
        at: at.to_string(),
        status: ReportStatus::Blocked.as_str().to_string(),
        text: format!("The cursor task prompt failed: {problem}."),
    }
}

pub fn accept_report(owner_required: bool, owner_exists: bool) -> bool {
    !owner_required || owner_exists
}

pub fn acp_mcp_server(exe: &Path, id: &str, inbox: &Path, owner: &Path) -> Value {
    json!({
        "name": SERVER_NAME,
        "command": exe.to_string_lossy(),
        "args": [
            "worker-mcp",
            "--id",
            id,
            "--inbox",
            inbox.to_string_lossy(),
            "--owner",
            owner.to_string_lossy()
        ],
        "env": []
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(store: Option<&str>) -> SessionRecord {
        SessionRecord {
            session_id: "s1".to_string(),
            cwd: PathBuf::from("/w"),
            inbox: PathBuf::from("/store/inbox"),
            store: store.map(PathBuf::from),
        }
    }

    #[test]
    fn session_record_round_trips_and_old_records_have_no_owner() {
        let owned = record(Some("/store"));
        assert_eq!(SessionRecord::parse(&owned.render()), Some(owned));
        let bare = r#"{"session_id":"s1","cwd":"/w","inbox":"/i"}"#;
        assert_eq!(SessionRecord::parse(bare).map(|r| r.store), Some(None));
        assert_eq!(SessionRecord::parse("nope"), None);
    }

    #[test]
    fn resume_requires_this_store_and_the_load_capability() {
        let store = Path::new("/store");
        assert_eq!(
            resume_decision("a", &record(Some("/store")), store, true),
            Resume::Go
        );
        for (rec, loads, why) in [
            (record(None), true, "no owner"),
            (record(Some("/other")), true, "/other"),
            (record(Some("/store")), false, "session/load"),
        ] {
            match resume_decision("a", &rec, store, loads) {
                Resume::Skip(message) => {
                    assert!(message.starts_with("cursor session a not resumed: "));
                    assert!(message.contains(why), "{message}");
                }
                Resume::Go => panic!("must skip"),
            }
        }
    }

    #[test]
    fn prompt_entries_sort_in_arrival_order() {
        assert!(prompt_entry_name(9, 5) < prompt_entry_name(10, 1));
    }

    #[test]
    fn version_output_is_parsed_without_the_build_hash() {
        assert_eq!(
            parse_agent_version("2026.10.14-0f9edde\n"),
            Some("2026.10.14")
        );
        assert_eq!(parse_agent_version("error: nope"), None);
        assert_eq!(parse_agent_version(""), None);
    }

    #[test]
    fn version_warning_only_when_the_version_differs() {
        assert_eq!(version_warning("2026.10.14-0f9edde"), None);
        assert!(version_warning("2026.11.01-abc")
            .unwrap()
            .contains("2026.11.01"));
        assert!(version_warning("???").unwrap().contains("could not read"));
    }

    #[test]
    fn only_protocol_one_is_accepted() {
        assert!(check_protocol(1).is_ok());
        assert!(check_protocol(2).unwrap_err().contains("protocol 2"));
    }

    #[test]
    fn auth_required_tells_the_user_to_log_in() {
        assert!(session_error(-32000, "x").contains("agent login"));
        assert!(session_error(-32603, "boom").contains("boom"));
    }

    #[test]
    fn mcp_server_entry_is_a_stdio_worker_mcp_command() {
        let entry = acp_mcp_server(
            Path::new("/bin/ralph"),
            "A",
            Path::new("/s/inbox"),
            Path::new("/s/cursor/A"),
        );
        assert_eq!(entry["name"], SERVER_NAME);
        assert_eq!(entry["command"], "/bin/ralph");
        assert_eq!(
            entry["args"],
            json!([
                "worker-mcp",
                "--id",
                "A",
                "--inbox",
                "/s/inbox",
                "--owner",
                "/s/cursor/A"
            ])
        );
        assert_eq!(entry["env"], json!([]));
    }

    #[test]
    fn a_report_needs_the_owner_file_only_when_one_is_required() {
        assert!(accept_report(false, false));
        assert!(accept_report(true, true));
        assert!(!accept_report(true, false));
    }

    #[test]
    fn spawn_handoff_prefers_error_then_ready_then_timeout() {
        let early = Duration::from_secs(1);
        assert_eq!(spawn_handoff(false, None, early), Handoff::Pending);
        assert_eq!(spawn_handoff(true, None, early), Handoff::Done);
        assert_eq!(
            spawn_handoff(true, Some("bad\n"), early),
            Handoff::Failed("bad".into())
        );
        assert_eq!(
            spawn_handoff(false, None, HANDOFF_TIMEOUT),
            Handoff::TimedOut
        );
        assert_eq!(spawn_handoff(true, None, HANDOFF_TIMEOUT), Handoff::Done);
    }

    #[test]
    fn close_handoff_is_done_once_the_owner_file_is_gone() {
        let early = Duration::from_secs(1);
        assert_eq!(close_handoff(true, early), Handoff::Pending);
        assert_eq!(close_handoff(false, early), Handoff::Done);
        assert_eq!(close_handoff(true, HANDOFF_TIMEOUT), Handoff::TimedOut);
    }

    #[test]
    fn requests_older_than_the_timeout_are_stale() {
        assert!(!request_stale(100, 129));
        assert!(request_stale(100, 130));
        assert!(!request_stale(100, 50));
    }

    fn facts(chat_alive: bool, group_alive: bool, mine: bool) -> OwnerFacts {
        OwnerFacts {
            chat_alive,
            group_alive,
            mine,
        }
    }

    #[test]
    fn live_owner_chat_is_waited_on_when_mine_and_busy_when_foreign() {
        assert_eq!(sweep_decision(facts(true, true, true)), Sweep::Wait);
        assert_eq!(sweep_decision(facts(true, true, false)), Sweep::Busy);
        assert_eq!(sweep_decision(facts(true, false, false)), Sweep::Busy);
    }

    #[test]
    fn dead_owner_chat_is_reclaimed_and_the_group_killed_only_if_it_is_the_same_process() {
        assert_eq!(
            sweep_decision(facts(false, true, false)),
            Sweep::Reclaim { kill_group: true }
        );
        assert_eq!(
            sweep_decision(facts(false, false, false)),
            Sweep::Reclaim { kill_group: false }
        );
    }

    #[test]
    fn owner_record_round_trips_and_rejects_garbage() {
        let record = OwnerRecord {
            chat_pid: 1,
            chat_start: "Mon Oct 15 10:00:00 2026".into(),
            group: 2,
            group_start: "Mon Oct 15 10:00:01 2026".into(),
        };
        assert_eq!(OwnerRecord::parse(&record.render()), Some(record));
        assert_eq!(OwnerRecord::parse("4242"), None);
    }

    fn options() -> Vec<PermissionOption> {
        vec![
            PermissionOption {
                option_id: "allow-once".into(),
                name: "Allow once".into(),
                kind: "allow_once".into(),
            },
            PermissionOption {
                option_id: "reject-once".into(),
                name: "Reject".into(),
                kind: "reject_once".into(),
            },
        ]
    }

    #[test]
    fn permission_content_reads_tool_summary_and_options() {
        let request = json!({
            "sessionId": "s",
            "toolCall": {"toolCallId": "t", "name": "shell", "kind": "execute", "title": "Run ls"},
            "options": [{"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"}]
        });
        let content = permission_content(&request);
        assert_eq!(content.tool, "shell");
        assert_eq!(content.summary, "Run ls");
        assert_eq!(content.options[0].option_id, "allow-once");
        let rendered: Value = serde_json::from_str(&content.render()).unwrap();
        assert_eq!(rendered["options"][0]["optionId"], "allow-once");
        assert_eq!(rendered["options"][0]["kind"], "allow_once");
    }

    #[test]
    fn permission_content_falls_back_when_fields_are_missing() {
        let content = permission_content(&json!({"toolCall": {"kind": "edit"}}));
        assert_eq!(content.tool, "edit");
        assert_eq!(content.summary, "permission requested");
        assert!(content.options.is_empty());
        assert_eq!(permission_content(&json!({})).tool, "tool");
    }

    #[test]
    fn match_option_takes_only_an_exact_option_id() {
        let options = options();
        assert_eq!(match_option(&options, "allow-once"), Some(&options[0]));
        assert_eq!(match_option(&options, " reject-once\n"), Some(&options[1]));
        assert_eq!(match_option(&options, &options[0].name), None);
        assert_eq!(match_option(&options, "Allow-Once"), None);
        assert_eq!(match_option(&options, "yes"), None);
        assert_eq!(match_option(&options, ""), None);
    }

    fn report_content(tool: &str, summary: &str) -> BlockedContent {
        BlockedContent {
            tool: tool.into(),
            summary: summary.into(),
            options: options(),
        }
    }

    #[test]
    fn only_the_worker_report_tool_is_auto_allowed() {
        let report = report_content("report", "ralph-worker-report: report");
        assert_eq!(
            auto_allow_option(&report).map(|o| o.option_id.as_str()),
            Some("allow-once")
        );
        let by_tool = report_content("ralph-worker-report: report", "x");
        assert!(auto_allow_option(&by_tool).is_some());
        assert!(auto_allow_option(&report_content("shell", "Run ls")).is_none());
        assert!(auto_allow_option(&report_content("x", "other-server-report: report")).is_none());
        assert!(auto_allow_option(&report_content("x", "ralph-chat-reply: reply")).is_none());
    }

    #[test]
    fn report_without_an_allow_once_option_still_waits() {
        let mut content = report_content("x", "ralph-worker-report: report");
        content.options.retain(|o| o.kind != "allow_once");
        assert!(auto_allow_option(&content).is_none());
    }

    #[test]
    fn caller_owns_worker_only_when_the_owner_is_an_ancestor() {
        assert!(caller_owns_worker(7078, &[71512, 71385, 7078, 1]));
        assert!(!caller_owns_worker(7078, &[71512, 71385, 9999, 1]));
        assert!(!caller_owns_worker(7078, &[]));
    }

    #[test]
    fn no_option_message_lists_every_option_id() {
        let message = no_option_message(&options());
        assert!(message.contains("allow-once, reject-once"));
    }

    #[test]
    fn only_a_user_turn_may_answer_a_permission() {
        assert!(permission_reply_allowed(Some(Trigger::User)).is_ok());
        assert!(permission_reply_allowed(Some(Trigger::Worker)).is_err());
        assert!(permission_reply_allowed(None).is_err());
    }

    #[test]
    fn task_prompt_is_the_worker_rules_then_the_task() {
        let prompt = task_prompt("do it");
        assert!(prompt.starts_with(WORKER_PROMPT));
        assert!(prompt.ends_with("\n\ndo it"));
    }

    #[test]
    fn turn_outcome_covers_every_branch() {
        let end = Ok("end_turn");
        assert_eq!(turn_outcome(end, true, false, false), TurnOutcome::Reported);
        assert_eq!(turn_outcome(end, false, false, false), TurnOutcome::Remind);
        assert_eq!(
            turn_outcome(end, false, true, false),
            TurnOutcome::Unreported
        );
        assert_eq!(
            turn_outcome(Ok("cancelled"), false, false, true),
            TurnOutcome::Closed
        );
        assert_eq!(
            turn_outcome(Err("x"), true, false, true),
            TurnOutcome::Closed
        );
        assert_eq!(
            turn_outcome(Ok("refusal"), false, false, false),
            TurnOutcome::Problem("the cursor agent ended its turn with `refusal`".into())
        );
        assert_eq!(
            turn_outcome(Err("boom"), false, false, false),
            TurnOutcome::Problem("boom".into())
        );
    }

    #[test]
    fn only_the_worker_report_tool_call_counts_as_reported() {
        let call = |kind: &str, title: &str| json!({"sessionUpdate": kind, "title": title});
        assert!(is_report_tool_call(&json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": "t1",
            "title": "ralph-worker: report",
            "rawInput": {"toolName": "report", "status": "done", "text": "hi"}
        })));
        assert!(!is_report_tool_call(&json!({
            "sessionUpdate": "tool_call",
            "toolCallId": "t1",
            "title": "MCP: tool",
            "kind": "other"
        })));
        assert!(!is_report_tool_call(&call(
            "tool_call",
            "ralph-worker: report"
        )));
        assert!(!is_report_tool_call(&call(
            "tool_call_update",
            "ralph-worker-report: report"
        )));
        assert!(!is_report_tool_call(&call(
            "tool_call_update",
            "other-server: report"
        )));
        assert!(!is_report_tool_call(&call(
            "tool_call_update",
            "ralph-worker: spawn"
        )));
        assert!(!is_report_tool_call(&call(
            "agent_message_chunk",
            "ralph-worker: report"
        )));
    }

    #[test]
    fn prompt_failure_report_blocks_the_worker() {
        let report = prompt_failure_report("A", "2026-01-01T00:00:00Z", "boom");
        assert_eq!(report.worker, "A");
        assert_eq!(report.status, "blocked");
        assert!(report.text.contains("boom"));
    }

    #[test]
    fn spawn_request_round_trips_through_json() {
        let request = SpawnRequest {
            worktree: "/w".into(),
            exe: "/bin/ralph".into(),
            inbox: "/s/inbox".into(),
            task: "do it".into(),
            at: 7,
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(&text).unwrap(),
            request
        );
    }
}
