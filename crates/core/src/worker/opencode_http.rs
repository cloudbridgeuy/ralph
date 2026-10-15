use super::{BlockedContent, PermissionOption};
use serde::Deserialize;
use serde_json::{json, Value};

const FALLBACK_FAILURE: &str = "opencode turn failed";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ServerEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub data: Value,
}

impl ServerEvent {
    pub fn session_id(&self) -> Option<&str> {
        self.data.get("sessionID").and_then(Value::as_str)
    }
}

pub fn parse_event(sse_line: &str) -> Option<ServerEvent> {
    let payload = sse_line.trim().strip_prefix("data:")?.trim();
    serde_json::from_str(payload).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    Succeeded,
    Failed(String),
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PermissionAsked {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub action: String,
    #[serde(default)]
    pub resources: Vec<String>,
    #[serde(default)]
    pub save: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionUpdate {
    TurnStarted,
    Blocked(PermissionAsked),
    Unblocked { request_id: String },
    TurnEnded(TurnEnd),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiError {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub message: Option<String>,
}

impl ApiError {
    fn describe(&self) -> String {
        self.message
            .clone()
            .or_else(|| self.kind.clone())
            .unwrap_or_else(|| FALLBACK_FAILURE.to_string())
    }
}

fn failure_message(error: Option<ApiError>) -> String {
    error
        .map(|error| error.describe())
        .unwrap_or_else(|| FALLBACK_FAILURE.to_string())
}

pub fn session_update(event: &ServerEvent) -> Option<SessionUpdate> {
    match event.kind.as_str() {
        "session.execution.started" => Some(SessionUpdate::TurnStarted),
        "session.execution.succeeded" => Some(SessionUpdate::TurnEnded(TurnEnd::Succeeded)),
        "session.execution.interrupted" => Some(SessionUpdate::TurnEnded(TurnEnd::Interrupted)),
        "session.execution.failed" => {
            let error = event
                .data
                .get("error")
                .and_then(|error| serde_json::from_value(error.clone()).ok());
            Some(SessionUpdate::TurnEnded(TurnEnd::Failed(failure_message(
                error,
            ))))
        }
        "permission.asked" => serde_json::from_value(event.data.clone())
            .ok()
            .map(SessionUpdate::Blocked),
        "permission.replied" => {
            event
                .data
                .get("requestID")
                .and_then(Value::as_str)
                .map(|request_id| SessionUpdate::Unblocked {
                    request_id: request_id.to_string(),
                })
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Once,
    Always,
    Reject,
}

impl PermissionDecision {
    fn wire(self) -> &'static str {
        match self {
            Self::Once => "once",
            Self::Always => "always",
            Self::Reject => "reject",
        }
    }
}

pub fn permission_decision(option_id: &str) -> Option<PermissionDecision> {
    [
        PermissionDecision::Once,
        PermissionDecision::Always,
        PermissionDecision::Reject,
    ]
    .into_iter()
    .find(|decision| decision.wire() == option_id.trim())
}

pub fn permission_reply_body(decision: PermissionDecision) -> Value {
    json!({ "decision": decision.wire() })
}

fn decision_option(decision: PermissionDecision, name: &str, kind: &str) -> PermissionOption {
    PermissionOption {
        option_id: decision.wire().to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
    }
}

pub fn opencode_blocked_content(asked: &PermissionAsked) -> BlockedContent {
    let summary = if asked.resources.is_empty() {
        asked.action.clone()
    } else {
        asked.resources.join("; ")
    };
    BlockedContent {
        tool: asked.action.clone(),
        summary,
        options: vec![
            decision_option(PermissionDecision::Once, "Allow once", "allow_once"),
            decision_option(PermissionDecision::Always, "Always allow", "allow_always"),
            decision_option(PermissionDecision::Reject, "Reject", "reject_once"),
        ],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiData<T> {
    pub data: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SessionMessage {
    User,
    Assistant {
        #[serde(default)]
        content: Vec<AssistantContent>,
        error: Option<ApiError>,
    },
    Idle {
        outcome: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AssistantContent {
    Text {
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnResult {
    pub end: TurnEnd,
    pub text: String,
}

fn turn_end(outcome: &str, turn: &[&SessionMessage]) -> TurnEnd {
    match outcome {
        "succeeded" => TurnEnd::Succeeded,
        "interrupted" => TurnEnd::Interrupted,
        "failed" => TurnEnd::Failed(failure_message(turn.iter().find_map(
            |message| match message {
                SessionMessage::Assistant { error, .. } => error.clone(),
                _ => None,
            },
        ))),
        other => TurnEnd::Failed(format!("unknown opencode outcome: {other}")),
    }
}

fn ends_aborted(turn: &[&SessionMessage]) -> bool {
    turn.iter()
        .find_map(|message| match message {
            SessionMessage::Assistant { error, .. } => Some(error.as_ref()),
            _ => None,
        })
        .flatten()
        .and_then(|error| error.kind.as_deref())
        == Some("aborted")
}

pub fn turn_result(newest_first: &[SessionMessage], active: bool) -> Option<TurnResult> {
    if active {
        return None;
    }
    let turn: Vec<&SessionMessage> = newest_first
        .iter()
        .take_while(|message| !matches!(message, SessionMessage::User))
        .collect();
    let outcome = turn.iter().find_map(|message| match message {
        SessionMessage::Idle { outcome } => Some(outcome.as_str()),
        _ => None,
    });
    let end = match outcome {
        Some(outcome) => turn_end(outcome, &turn),
        None if ends_aborted(&turn) => TurnEnd::Interrupted,
        None => return None,
    };
    let text = turn
        .iter()
        .rev()
        .filter_map(|message| match message {
            SessionMessage::Assistant { content, .. } => Some(content),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            AssistantContent::Text { text } => Some(text.as_str()),
            AssistantContent::Other => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(TurnResult { end, text })
}

pub fn create_session_body(directory: &str, model: &str) -> Result<Value, String> {
    let (provider, id) = model
        .split_once('/')
        .filter(|(provider, id)| !provider.is_empty() && !id.is_empty())
        .ok_or_else(|| format!("model must look like provider/id, got: {model}"))?;
    Ok(json!({
        "location": { "directory": directory },
        "model": { "id": id, "providerID": provider },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::match_option;

    const SESSION: &str = "ses_ebb06fb5dffePK4O3B4WCKFy5g";
    const PERMISSION_ID: &str = "per_144f90966001W6ctCffcyHF3La";
    const STARTED: &str = r#"{"id":"evt_144f90510001tcl9eC2hYLiS5A","created":1792158532880,"type":"session.execution.started","data":{"sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g"},"durable":{"aggregateID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","seq":2,"version":1}}"#;
    const SUCCEEDED: &str = r#"{"id":"evt_144f70aa6001gPnrN7oUY5oLif","created":1792158403238,"type":"session.execution.succeeded","data":{"sessionID":"ses_ralphguz581test0001"},"durable":{"aggregateID":"ses_ralphguz581test0001","seq":20,"version":1}}"#;
    const INTERRUPTED_USER: &str = r#"{"id":"evt_144f9d3f3002PV91BcsEi1g5Ga","created":1792158585843,"type":"session.execution.interrupted","data":{"sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","reason":"user"},"durable":{"aggregateID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","seq":25,"version":1}}"#;
    const INTERRUPTED_SHUTDOWN: &str = r#"{"id":"evt_144f96924001JaCMQWLZT3Talc","created":1792158558500,"type":"session.execution.interrupted","data":{"sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","reason":"shutdown"},"durable":{"aggregateID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","seq":13,"version":1}}"#;
    const ASKED: &str = r#"{"id":"evt_144f909660026JBJNG1DCSCCtm","created":1792158533990,"type":"permission.asked","location":{"directory":"/tmp/oc581/w"},"data":{"id":"per_144f90966001W6ctCffcyHF3La","sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","action":"shell","resources":["touch made581.txt"],"save":["touch *"],"source":{"type":"tool","messageID":"msg_144f9051b001Ee1jNz3t6Jd7r0","id":"toolu_01RHicj7koJxfoHqi9Asz31u"}}}"#;
    const REPLIED: &str = r#"{"id":"evt_144f96920001k36iPL3Pf2iK2n","created":1792158558496,"type":"permission.replied","location":{"directory":"/tmp/oc581/w"},"data":{"sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","requestID":"per_144f90966001W6ctCffcyHF3La","reply":"reject"}}"#;
    const IGNORED: &str = r#"{"id":"evt_144f90901001AaAc5p6CN2klgh","created":1792158533889,"type":"session.step.started","location":{"directory":"/tmp/oc581/w"},"data":{"sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","agent":"build","model":{"id":"claude-haiku-4.5","providerID":"github-copilot","variant":"default"},"assistantMessageID":"msg_144f9051b001Ee1jNz3t6Jd7r0","started":1792158532895},"durable":{"aggregateID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","seq":6,"version":1}}"#;
    const PERMISSION_LIST: &str = r#"{"data":[{"id":"per_144f90966001W6ctCffcyHF3La","sessionID":"ses_ebb06fb5dffePK4O3B4WCKFy5g","action":"shell","resources":["touch made581.txt"],"save":["touch *"],"source":{"type":"tool","messageID":"msg_144f9051b001Ee1jNz3t6Jd7r0","id":"toolu_01RHicj7koJxfoHqi9Asz31u"}}]}"#;
    const MESSAGES_OK: &str = r#"{"data":[{"id":"msg_144fa6fc6001pqJVuvucV8jwBr","type":"idle","outcome":"succeeded"},{"id":"msg_144fa6b6e0010I3ooSoYCCgDaB","type":"assistant","content":[{"type":"text","text":"DONE."}],"finish":"stop"},{"id":"msg_144fa63a1001y6GaFMDlT0tBvW","type":"assistant","content":[{"type":"text","text":"I'll call the ralph-worker report tool with the parameters you specified."},{"type":"tool","id":"toolu_01PCKXZNANSVSJkWySxuR2qf","name":"execute"}],"finish":"tool-calls"},{"id":"msg_144fa639e0013o9DVE725FraI5","metadata":{"notice":"instructions","instructionSources":["core/codemode"]},"type":"system","text":"The Code Mode tool catalog has changed. This catalog supersedes the previous Code Mode tool catalog.\n\n# Code Mode\n\nUse the `execute` tool to call the tools listed below. They cannot be called directly, and neither can `search`. Both only work inside code you pass to `execute`.\n\nThe catalog is partial. Inside `execute`, use `search(...)` to find a tool, then call it by the `path` in the result. `search` is synchronous. Call it without `await`; it does not return a Promise. Do not guess tool names.\n\n- search({\n  query?: string,\n  namespace?: string,\n  /** @integer @exclusiveMinimum 0 */\n  limit?: number,\n  /** @integer @minimum 0 */\n  offset?: number,\n}): {\n  items: Array<{\n      path: string,\n      description: string,\n      signature: string,\n    }>,\n  /** @integer @minimum 0 */\n  remaining: number,\n  next: {\n      /** @integer @minimum 0 */\n      offset: number,\n    } | null,\n}\n\n## Available tools\n\n- mcptools (85 tools, 7 shown)\n  - tools.mcptools.atlas_status(): Promise<{\n  format: string,\n  freshness?: string | null,\n  text: string,\n}> // Check Atlas readiness, structural freshness and optional enrichment before navigation. No model calls. Absent index r...\n  - tools.mcptools.jira_query_delete({\n  name: string,\n}): Promise<{\n  message: string,\n  status: string,\n}> // Delete a saved Jira query by name. Removes the query from ~/.config/mcptools/queries/\n  - tools.mcptools.jira_query_list(): Promise<{\n  queries: Array<string>,\n}> // List all saved Jira queries. Returns a list of query names stored in ~/.config/mcptools/queries/\n  - tools.mcptools.jira_query_load({\n  name: string,\n}): Promise<{\n  name: string,\n  query: string,\n}> // Load and display the contents of a saved Jira query. Returns the query name and the JQL query text.\n  - tools.mcptools.linear_auth_status(): Promise<{\n  email?: string | null,\n  id: string,\n  name: string,\n}> // Show the Linear viewer identity for LINEAR_API_KEY. Returns id, name, and email. Requires LINEAR_API_KEY environment ...\n  - tools.mcptools.ui_annotations_clear({\n  url?: string,\n}): Promise<{\n  /** @format uint64 @integer @minimum 0 */\n  cleared: number,\n}> // Clear all UI annotations from the dev server.\n  - tools.mcptools.ui_annotations_resolve({\n  id: string,\n  summary: string,\n  url?: string,\n}): Promise<{\n  id: string,\n  resolved: boolean,\n}> // Mark a UI annotation as resolved with a summary of the changes made.\n- opencode (8 tools) // Tools for managing OpenCode itself, such as working with sessions and worktrees, searching the available models, and reading MCP resources.\n  - tools.opencode.list_mcp_resources({\n  /** MCP server name as configured. Omit to list every server. */\n  server?: string,\n}): Promise<{\n  resources: Array<{\n      server: string,\n      name: string,\n      uri: string,\n      description?: string,\n      mimeType?: string,\n    }>,\n  templates: Array<{\n      server: string,\n      name: string,\n      uriTemplate: string,\n      description?: string,\n      mimeType?: string,\n    }>,\n}> // List documents, records, and other data exposed by MCP servers. Use this when the user refers to something that is no...\n  - tools.opencode.models({\n  /** Text to search for in model names and IDs. */\n  query?: string,\n  /** Provider ID or name to filter by. Try your own provider first. */\n  provider?: string,\n  /** Include older versions of each model family. By default only the newest version is listed. */\n  all?: boolean,\n  /** Maximum number of models to return. Defaults to 20. @integer @minimum 1 @maximum 100 */\n  limit?: number,\n  /** Number of models to skip, for paging through results. @integer @minimum 0 */\n  offset?: number,\n}): Promise<{\n  /** Matching models grouped by provider. Your own provider comes first. */\n  providers: Array<{\n      id: string,\n      name: string,\n      /** Newest first. */\n      models: Array<{\n          /** providerID/modelID */\n          id: string,\n          name: string,\n          /** Release date as a Unix timestamp in milliseconds, or 0 when unknown. */\n          released: number,\n          variants: Array<string>,\n          /** Pricing in USD per million tokens. */\n          cost: Array<{\n              tier?: {\n                type: \"context\",\n                /** @integer */\n                size: number,\n              },\n              input: number,\n              output: number,\n              cache: {\n                read: number,\n                write: number,\n              },\n            }>,\n          status: \"alpha\" | \"beta\" | \"deprecated\" | \"active\",\n        }>,\n    }>,\n  /** Number of matching models across all pages. @integer */\n  total: number,\n  /** Offset of the next page, or null on the last page. */\n  next: number | null,\n}> // Search the models available to use. Use this to turn a model name the user mentions into an exact reference before ru...\n  - tools.opencode.read_mcp_resource({\n  /** The server field of the discovered resource. */\n  server: string,\n  /** Exact resource URI from discovery, an expanded uriTemplate, or the user. */\n  uri: string,\n}): Promise<{\n  server: string,\n  uri: string,\n  contents: Array<{\n        type: \"text\",\n        uri: string,\n        text: string,\n        mimeType?: string,\n      } | {\n        type: \"blob\",\n        uri: string,\n        blob: string,\n        mimeType?: string,\n      }>,\n}> // Read one MCP resource by server and URI. Not for local files. Always return the full contents rather than slicing or ...\n  - tools.opencode.session_move({\n  /** Omit to move the current session. @pattern ^ses */\n  sessionID?: string,\n  /** Destination directory, relative to the target session's directory or absolute. Supports ~. @minLength 1 */\n  directory: string,\n}): Promise<{\n  /** @pattern ^ses */\n  sessionID: string,\n  directory: string,\n}> // Move a session to another directory, or omit sessionID to move the current session. The current session moves at the ...\n  - tools.opencode.session_rename({\n  /** Omit to rename the current session. @pattern ^ses */\n  sessionID?: string,\n  /** New session title. @minLength 1 */\n  title: string,\n}): Promise<{\n  /** @pattern ^ses */\n  sessionID: string,\n  title: string,\n}> // Rename a session, or omit sessionID to rename the current session. Use a short, specific title that summarizes the wo...\n  - tools.opencode.worktree_create({\n  name: string,\n}): Promise<{\n  directory: string,\n}> // Create a named worktree. Returns its directory.\n  - tools.opencode.worktree_list(): Promise<{\n  worktrees: Array<{\n      directory: string,\n      strategy?: string,\n    }>,\n}> // List a repository's worktrees.\n  - tools.opencode.worktree_remove({\n  /** Absolute path of the worktree to remove. @minLength 1 */\n  directory: string,\n  /** Discard modified and untracked files. Only set this when the user has confirmed forced removal. */\n  force?: boolean,\n}): Promise<{\n  directory: string,\n  removed: boolean,\n}> // Remove a worktree. The repository location selects configuration; directory identifies the worktree to remove.\n- playwright (25 tools, 7 shown)\n  - tools.playwright.browser_close(): Promise<unknown> // Close the page\n  - tools.playwright.browser_file_upload({\n  /**\n   * The absolute paths to the files to upload. Can be single file or multiple files. If omitted, file chooser is cancelled.\n   */\n  paths?: Array<string>,\n}): Promise<unknown> // Upload one or multiple files\n  - tools.playwright.browser_handle_dialog({\n  /** Whether to accept the dialog. */\n  accept: boolean,\n  /** The text of the prompt in case of a prompt dialog. */\n  promptText?: string,\n}): Promise<unknown> // Handle a dialog\n  - tools.playwright.browser_navigate({\n  /** The URL to navigate to */\n  url: string,\n}): Promise<unknown> // Navigate to a URL\n  - tools.playwright.browser_navigate_back(): Promise<unknown> // Go back to the previous page in the history\n  - tools.playwright.browser_press_key({\n  /** Name of the key to press or a character to generate, such as `ArrowLeft` or `a` */\n  key: string,\n}): Promise<unknown> // Press a key on the keyboard\n  - tools.playwright.browser_resize({\n  /** Width of the browser window */\n  width: number,\n  /** Height of the browser window */\n  height: number,\n}): Promise<unknown> // Resize the browser window\n- ralph-worker (1 tool)\n  - tools[\"ralph-worker\"].report({\n  /** done: task finished. blocked: you cannot go on. question: you need a decision. */\n  status: \"done\" | \"blocked\" | \"question\",\n  /** Full summary: what you did and what you found, or what blocks you, or what you need decided. */\n  text: string,\n}): Promise<unknown> // Send a message to the person who dispatched you. Call it at the end of every turn: when you finish, get blocked, or n...","description":"Instructions updated: core/codemode"},{"id":"msg_144fa5c68001776Ck54jVBMLVt","type":"assistant","content":[{"type":"text","text":"I'll call the ralph-worker report tool with the parameters you specified."},{"type":"tool","id":"toolu_01NppButZgLWQ7yPYoMP6Gmw","name":"shell"}],"finish":"tool-calls"},{"id":"msg_144fa597f001D8XmKnjlgleoYa","text":"Call the ralph-worker report tool with status done and text hello581. Then reply DONE.","type":"user"}]}"#;
    const MESSAGES_INTERRUPTED: &str = r#"{"data":[{"id":"msg_144faee7b002PYDfKHvDC9Lm9p","type":"idle","outcome":"interrupted"},{"id":"msg_144fad697001mKCOIdt1BoQd0n","type":"assistant","content":[{"type":"tool","id":"toolu_01YbdNV7KVpuVgentiDLcHSY","name":"shell"}],"finish":"error","error":{"type":"aborted","message":"Step interrupted"}},{"id":"msg_144fad68e0014YEI5YyiJQYgY5","text":"Run `sleep 20` with your bash tool then reply DONE","type":"user"},{"id":"msg_144f9d3f3002PV91BcsEi1g5Ga","type":"idle","outcome":"interrupted"},{"id":"msg_144f998730016gxer0mIkK2tj6","type":"assistant","content":[{"type":"tool","id":"toolu_01Gpo3EhxgGcfJQSkmkzEWes","name":"shell"}],"finish":"error","error":{"type":"aborted","message":"Step interrupted"}},{"id":"msg_144f99866001PnvZpj7zrtXHBb","text":"Run the shell command `sleep 30 && touch late581.txt` with your bash tool, then reply DONE","type":"user"},{"id":"msg_144f9051b001Ee1jNz3t6Jd7r0","type":"assistant","content":[{"type":"tool","id":"toolu_01RHicj7koJxfoHqi9Asz31u","name":"shell"}],"finish":"error","error":{"type":"aborted","message":"Step interrupted"}},{"id":"msg_144f9050e00172FEtoa181cb7f","text":"Run the shell command `touch made581.txt` with your bash tool, then reply with exactly: DONE","type":"user"}]}"#;
    const FAILED_AUTH_2023: &str = r#"{"id":"evt_14156a74f0016bXIBSjFoIkf0E","created":1792097560399,"type":"session.execution.failed","data":{"sessionID":"ses_ebea95bf2ffel2McK50uE4e7nB","error":{"type":"provider.auth","message":"OAuth access token is invalid.","status":401}}}"#;
    const FAILED_NO_MESSAGE_2023: &str = r#"{"id":"evt_1","type":"session.execution.failed","data":{"sessionID":"ses_x","error":{"type":"provider.auth"}}}"#;
    const FAILED_NO_ERROR_2023: &str =
        r#"{"id":"evt_2","type":"session.execution.failed","data":{"sessionID":"ses_x"}}"#;
    const MESSAGES_AUTH_2023: &str = r#"{"data":[{"id":"msg_a","type":"idle","outcome":"failed"},{"id":"msg_b","type":"assistant","content":[],"finish":"error","error":{"type":"provider.auth","message":"OAuth access token is invalid.","status":401}},{"id":"msg_c","text":"Run","type":"user"}],"cursor":{}}"#;
    const MESSAGES_NO_ROUTE_2023: &str = r#"{"data":[{"id":"msg_a","type":"idle","outcome":"failed"},{"id":"msg_c","text":"Run","type":"user"}],"cursor":{}}"#;
    const MESSAGES_RUNNING: &str = r#"{"data":[{"id":"msg_5","type":"assistant","content":[{"type":"text","text":"working"}]},{"id":"msg_4","text":"go","type":"user"},{"id":"msg_1","type":"idle","outcome":"succeeded"}],"cursor":{}}"#;

    fn event(line: &str) -> ServerEvent {
        parse_event(&format!("data: {line}")).unwrap()
    }

    fn messages(body: &str) -> Vec<SessionMessage> {
        serde_json::from_str::<ApiData<Vec<SessionMessage>>>(body)
            .unwrap()
            .data
    }

    fn asked() -> PermissionAsked {
        match session_update(&event(ASKED)) {
            Some(SessionUpdate::Blocked(asked)) => asked,
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn parse_event_reads_data_lines_only() {
        let parsed = parse_event(&format!("data: {STARTED}")).unwrap();
        assert_eq!(parsed.kind, "session.execution.started");
        assert_eq!(parsed.id, "evt_144f90510001tcl9eC2hYLiS5A");
        assert_eq!(parse_event(&format!("data:{STARTED}")), Some(parsed));
        assert_eq!(parse_event(": keepalive"), None);
        assert_eq!(parse_event("event: message"), None);
        assert_eq!(parse_event("data: not json"), None);
        assert_eq!(parse_event(""), None);
    }

    #[test]
    fn session_id_routes_events_without_location() {
        assert_eq!(event(STARTED).session_id(), Some(SESSION));
        assert_eq!(
            event(FAILED_AUTH_2023).session_id(),
            Some("ses_ebea95bf2ffel2McK50uE4e7nB")
        );
        let other = event(r#"{"id":"evt_s","type":"server.connected","data":{}}"#);
        assert_eq!(other.session_id(), None);
    }

    #[test]
    fn session_update_maps_turn_events() {
        assert_eq!(
            session_update(&event(STARTED)),
            Some(SessionUpdate::TurnStarted)
        );
        assert_eq!(
            session_update(&event(SUCCEEDED)),
            Some(SessionUpdate::TurnEnded(TurnEnd::Succeeded))
        );
        for line in [INTERRUPTED_USER, INTERRUPTED_SHUTDOWN] {
            assert_eq!(
                session_update(&event(line)),
                Some(SessionUpdate::TurnEnded(TurnEnd::Interrupted))
            );
        }
        assert_eq!(session_update(&event(IGNORED)), None);
    }

    #[test]
    fn session_update_failed_prefers_message_then_type_then_fallback() {
        let failed = |line| match session_update(&event(line)) {
            Some(SessionUpdate::TurnEnded(TurnEnd::Failed(message))) => message,
            other => panic!("expected Failed, got {other:?}"),
        };
        assert_eq!(failed(FAILED_AUTH_2023), "OAuth access token is invalid.");
        assert_eq!(failed(FAILED_NO_MESSAGE_2023), "provider.auth");
        assert_eq!(failed(FAILED_NO_ERROR_2023), FALLBACK_FAILURE);
    }

    #[test]
    fn session_update_maps_permission_events() {
        let asked = asked();
        assert_eq!(asked.id, PERMISSION_ID);
        assert_eq!(asked.session_id, SESSION);
        assert_eq!(asked.action, "shell");
        assert_eq!(asked.resources, vec!["touch made581.txt"]);
        assert_eq!(asked.save, vec!["touch *"]);
        assert_eq!(
            session_update(&event(REPLIED)),
            Some(SessionUpdate::Unblocked {
                request_id: PERMISSION_ID.to_string()
            })
        );
    }

    #[test]
    fn permission_list_parses_like_the_event() {
        let list: ApiData<Vec<PermissionAsked>> = serde_json::from_str(PERMISSION_LIST).unwrap();
        assert_eq!(list.data, vec![asked()]);
    }

    #[test]
    fn blocked_content_round_trips_through_match_option() {
        let content = opencode_blocked_content(&asked());
        assert_eq!(content.tool, "shell");
        assert_eq!(content.summary, "touch made581.txt");
        let kinds: Vec<&str> = content.options.iter().map(|o| o.kind.as_str()).collect();
        assert_eq!(kinds, vec!["allow_once", "allow_always", "reject_once"]);
        let picked = match_option(&content.options, " always ").unwrap();
        assert_eq!(picked.kind, "allow_always");
        assert_eq!(
            permission_decision(&picked.option_id),
            Some(PermissionDecision::Always)
        );
        assert!(match_option(&content.options, "maybe").is_none());
    }

    #[test]
    fn blocked_content_without_resources_names_the_action() {
        let mut bare = asked();
        bare.resources.clear();
        assert_eq!(opencode_blocked_content(&bare).summary, "shell");
        bare.resources = vec!["a".to_string(), "b".to_string()];
        assert_eq!(opencode_blocked_content(&bare).summary, "a; b");
    }

    #[test]
    fn decisions_parse_and_build_bodies() {
        assert_eq!(permission_decision("once"), Some(PermissionDecision::Once));
        assert_eq!(
            permission_decision(" reject\n"),
            Some(PermissionDecision::Reject)
        );
        assert_eq!(permission_decision("allow_once"), None);
        assert_eq!(
            permission_reply_body(PermissionDecision::Once),
            json!({"decision": "once"})
        );
        assert_eq!(
            permission_reply_body(PermissionDecision::Always),
            json!({"decision": "always"})
        );
        assert_eq!(
            permission_reply_body(PermissionDecision::Reject),
            json!({"decision": "reject"})
        );
    }

    #[test]
    fn turn_result_succeeded_collects_text() {
        assert_eq!(
            turn_result(&messages(MESSAGES_OK), false),
            Some(TurnResult {
                end: TurnEnd::Succeeded,
                text: "I'll call the ralph-worker report tool with the parameters you specified.\nI'll call the ralph-worker report tool with the parameters you specified.\nDONE.".to_string()
            })
        );
    }

    #[test]
    fn turn_result_counts_only_the_newest_turn() {
        assert_eq!(
            turn_result(&messages(MESSAGES_INTERRUPTED), false),
            Some(TurnResult {
                end: TurnEnd::Interrupted,
                text: String::new()
            })
        );
    }

    #[test]
    fn turn_result_rejected_turn_without_idle_is_interrupted_when_inactive() {
        let all = messages(MESSAGES_INTERRUPTED);
        let rejected = &all[6..];
        assert!(matches!(rejected[0], SessionMessage::Assistant { .. }));
        assert_eq!(
            turn_result(rejected, false),
            Some(TurnResult {
                end: TurnEnd::Interrupted,
                text: String::new()
            })
        );
        assert_eq!(turn_result(rejected, true), None);
    }

    #[test]
    fn turn_result_without_idle_needs_an_aborted_assistant() {
        let failed_step = vec![
            SessionMessage::Assistant {
                content: vec![],
                error: Some(ApiError {
                    kind: Some("provider.auth".to_string()),
                    message: None,
                }),
            },
            SessionMessage::User,
        ];
        assert_eq!(turn_result(&failed_step, false), None);
        assert_eq!(turn_result(&[SessionMessage::User], false), None);
    }

    #[test]
    fn turn_result_joins_text_in_chronological_order() {
        let ordered = vec![
            SessionMessage::Idle {
                outcome: "succeeded".to_string(),
            },
            SessionMessage::Assistant {
                content: vec![AssistantContent::Text {
                    text: "two".to_string(),
                }],
                error: None,
            },
            SessionMessage::Other,
            SessionMessage::Assistant {
                content: vec![
                    AssistantContent::Other,
                    AssistantContent::Text {
                        text: "one".to_string(),
                    },
                ],
                error: None,
            },
            SessionMessage::User,
        ];
        assert_eq!(turn_result(&ordered, false).unwrap().text, "one\ntwo");
    }

    #[test]
    fn turn_result_failed_uses_assistant_error_or_fallback() {
        assert_eq!(
            turn_result(&messages(MESSAGES_AUTH_2023), false)
                .unwrap()
                .end,
            TurnEnd::Failed("OAuth access token is invalid.".to_string())
        );
        assert_eq!(
            turn_result(&messages(MESSAGES_NO_ROUTE_2023), false)
                .unwrap()
                .end,
            TurnEnd::Failed(FALLBACK_FAILURE.to_string())
        );
    }

    #[test]
    fn turn_result_unknown_outcome_fails_naming_it() {
        let odd = vec![SessionMessage::Idle {
            outcome: "paused".to_string(),
        }];
        assert_eq!(
            turn_result(&odd, false).unwrap().end,
            TurnEnd::Failed("unknown opencode outcome: paused".to_string())
        );
    }

    #[test]
    fn turn_result_is_none_while_running() {
        assert_eq!(turn_result(&messages(MESSAGES_RUNNING), false), None);
        assert_eq!(turn_result(&messages(MESSAGES_OK), true), None);
        assert_eq!(turn_result(&[], false), None);
    }

    #[test]
    fn unknown_message_types_are_tolerated() {
        let parsed = messages(
            r#"{"data":[{"id":"m","type":"compaction"},{"id":"n","type":"assistant","content":[{"type":"image"}]}]}"#,
        );
        assert_eq!(parsed[0], SessionMessage::Other);
        assert_eq!(
            parsed[1],
            SessionMessage::Assistant {
                content: vec![AssistantContent::Other],
                error: None
            }
        );
    }

    #[test]
    fn create_session_body_splits_model_on_first_slash() {
        assert_eq!(
            create_session_body("/tmp/oc580/w", "openai/gpt-6.1-sol").unwrap(),
            json!({
                "location": {"directory": "/tmp/oc580/w"},
                "model": {"id": "gpt-6.1-sol", "providerID": "openai"}
            })
        );
        assert_eq!(
            create_session_body("/w", "openrouter/meta/llama").unwrap()["model"],
            json!({"id": "meta/llama", "providerID": "openrouter"})
        );
    }

    #[test]
    fn create_session_body_rejects_bad_models() {
        for bad in ["gpt", "/gpt", "openai/", ""] {
            assert!(create_session_body("/w", bad).is_err(), "{bad}");
        }
    }
}
