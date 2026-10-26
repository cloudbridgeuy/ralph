use crate::chat_memory::SERVER_NAME;
use crate::chat_thread::ResetReason;
use crate::stream::IterationCosts;
use crate::worker::ATTACHMENTS_DIR;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

pub const VIEW_LINES: usize = 200;
pub const RECENT_HISTORY_TOKENS: usize = 8192;
pub const OLDER_VIEW_STEP: usize = 32;
pub const WORKER_CHECK_SECS: u64 = 60;
pub const WORKER_QUIET_SECS: u64 = 300;
pub const WATCH_TIMEOUT_SECS: u64 = 3600;
pub const RESET_TURNS: u64 = 30;
pub const RESET_TOKENS: u64 = 140_000;
const BLOCKING_LIMIT: &str = "blocking_limit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetLimits {
    pub turns: u64,
    pub tokens: u64,
}

pub fn hit_blocking_limit(costs: &IterationCosts) -> bool {
    costs.terminal_reason.as_deref() == Some(BLOCKING_LIMIT)
}

pub fn reset_reason(
    limits: &ResetLimits,
    turns_on_process: u64,
    costs: &IterationCosts,
) -> Option<ResetReason> {
    if turns_on_process >= limits.turns {
        return Some(ResetReason::Turns);
    }
    let over_tokens = costs
        .context_tokens
        .is_some_and(|tokens| tokens >= limits.tokens);
    (over_tokens || hit_blocking_limit(costs)).then_some(ResetReason::Tokens)
}

pub fn resolve_worker_check_secs(config: Option<u64>) -> Option<u64> {
    match config.unwrap_or(WORKER_CHECK_SECS) {
        0 => None,
        secs => Some(secs),
    }
}

pub fn resolve_watch_timeout(config: Option<u64>) -> Result<Duration, String> {
    match config.unwrap_or(WATCH_TIMEOUT_SECS) {
        0 => Err("watch_timeout_secs must be above 0.".to_string()),
        secs => Ok(Duration::from_secs(secs)),
    }
}

pub fn resolve_wrap_width(
    flag: Option<usize>,
    env: Option<&str>,
    config: Option<usize>,
) -> Result<Option<usize>, String> {
    let env = env
        .map(|value| {
            value
                .trim()
                .parse()
                .map_err(|_| format!("wrap width {value:?} is not a column count."))
        })
        .transpose()?;
    Ok(flag.or(env).or(config).filter(|&width| width > 0))
}

pub fn resolve_recent_history_tokens(flag: Option<usize>, config: Option<usize>) -> usize {
    flag.or(config).unwrap_or(RECENT_HISTORY_TOKENS)
}

pub const SYSTEM_PROMPT: &str = "\
You are in a long-running chat with one person. The chat so far is compressed, oldest first. The older lines are at the end of this system prompt. The most recent lines start the message you receive, then the new message.

Lines that start with #<id> are summaries of older parts of the chat. The zoom tool opens one into its two halves. The recall tool searches the full chat by regex, newest first. The search tool ranks messages for words, \"quoted phrases\", and identifiers like GUZ-709, and can filter by role and date. The read tool pages through a range of ids in order, as original text or leaf summaries.

Before you guess or ask the person for a fact, use zoom, recall, and search until the facts are whole.

Messages that start with [worker <id> · <status>] come from agents working for the person, not from the person. Reply to a worker yourself only when the answer is already settled in the chat: a decision the person made, scope they approved, or a step in a plan they agreed. Merges, pushes, PRs, Linear status changes, new scope, and anything public always go to the person: tell them what the worker did or needs, with your recommendation, and wait. List every reply you sent to a worker in your next message to the person. Use spawn_worker and close_worker only to carry out what the person asked in their latest message; set abandon on close_worker only when the person said to drop the work, and force only when the person said to close a worker of another chat.

Messages that start with [worker watch:<name> · watch] are watch reports: a command you started with the watch tool ended, and they come from that watch, not from an agent or the person. Before calling watch, show the person the exact command and wait for their approval. Act on the follow-up instruction in the report.

Reply format: lead with the answer, in 3 lines if it fits. Flag each risk in one line. End with one recommended next step, not a list of options. When the reply will end with a Next step the user can say, call `suggest_reply` first, before you write any reply text, with the exact quoted words of that line. Then write the reply once and never mention the suggestion.

Ralph help: when the person asks how ralph chat itself works (its commands, keys, or status bar), call `help` first and answer from its text, not from the ralph source.

Ralph config: when the person asks about ralph's settings or asks to change how ralph behaves or looks, call `help` with topic `config`, then `config <section>`, before you answer; propose a code change only when no setting covers it. Before you edit config.toml, copy it to config.toml.bak in the same directory. After editing, run `ralph config check`; if it fails, copy config.toml.bak back over config.toml and report the error.

Ask first: when two readings of a request lead to different work, show a 3-5 line plan and wait for a go.

Verify: never state as fact what you have not checked. A worker or subagent \"done\" is unverified until you check its diff or test output, and the git and Linear state.

Cite history: when you state a fact from older chat, check it with zoom or recall, name its #<id>, and say if it came from a summary.

Development roles: you never do the work yourself; you scope it, decide, and accept results. Read inline only to scope, with at most 3 tool calls per question. Reads across several files, broad searches, and test runs go to the evidence-collector subagent. All code edits go to the implementer subagent. Fixing or finishing work that fell short goes to the reviser subagent. Every dispatch prompt states the goal, the exact question, known facts, the files the worker owns (all other code is read-only), output format and size cap, stop condition and tool budget, \"Report not-found, do not widen scope.\", and \"You are a subagent. Don't run mem.\" After 2 failed repair rounds on one issue, diagnose before you dispatch again; after a 3rd, stop and wait for the person.

Forget requests: you have no erase tool. Say so plainly; never summarize the item as resolved.

The chat is logged automatically. Never claim to save, store, or remember anything manually.";

pub fn turn_system_prompt(older: &[String]) -> String {
    if older.is_empty() {
        return SYSTEM_PROMPT.to_string();
    }
    format!(
        "{SYSTEM_PROMPT}\n\nOlder chat, compressed, oldest first:\n{}",
        older.join("\n")
    )
}

pub fn turn_prompt(view_lines: &[String], message: &str) -> String {
    format!("{}\n\nNew message:\n{message}", view_lines.join("\n"))
}

pub struct ImportedServers {
    pub servers: Map<String, Value>,
    pub missing: Vec<String>,
}

pub fn select_servers(claude_config: &Value, names: &[String]) -> ImportedServers {
    let mut servers = Map::new();
    let mut missing = Vec::new();
    for name in names {
        match claude_config["mcpServers"].get(name) {
            Some(server) if name != SERVER_NAME => {
                servers.insert(name.clone(), server.clone());
            }
            _ => missing.push(name.clone()),
        }
    }
    ImportedServers { servers, missing }
}

pub fn mcp_config(exe: &Path, store: &Path, socket: &Path, imported: &Map<String, Value>) -> Value {
    let args = vec![
        "chat-mcp".to_string(),
        "--store".to_string(),
        store.to_string_lossy().into_owned(),
        "--socket".to_string(),
        socket.to_string_lossy().into_owned(),
    ];
    let mut servers = imported.clone();
    servers.insert(
        SERVER_NAME.to_string(),
        json!({ "command": exe.to_string_lossy(), "args": args }),
    );
    json!({ "mcpServers": servers })
}

pub fn build_mcp_args(config_path: &Path, config: &Value) -> Vec<String> {
    vec![
        "--mcp-config".to_string(),
        config_path.to_string_lossy().into_owned(),
        "--allowedTools".to_string(),
        allowed_tools(config),
        "--strict-mcp-config".to_string(),
    ]
}

fn allowed_tools(config: &Value) -> String {
    config["mcpServers"]
        .as_object()
        .into_iter()
        .flat_map(Map::keys)
        .map(|name| format!("mcp__{name}__*"))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn build_nap_args(store: &Path) -> Vec<OsString> {
    vec![
        "chat-nap".into(),
        "--store".into(),
        store.as_os_str().to_owned(),
    ]
}

pub fn build_attachments_args(store_dir: &Path) -> Vec<String> {
    vec![
        "--add-dir".to_string(),
        store_dir
            .join(ATTACHMENTS_DIR)
            .to_string_lossy()
            .into_owned(),
    ]
}

pub struct Subagent {
    name: &'static str,
    description: &'static str,
    model: &'static str,
    prompt: &'static str,
}

pub const SUBAGENTS: [Subagent; 3] = [
    Subagent {
        name: "evidence-collector",
        description: "Collects bounded, read-only evidence: multi-file reads, broad searches, and test runs.",
        model: "haiku",
        prompt: "Collect the requested evidence with read-only inspection. Report relevant files, commands, outputs, and uncertainty. Do not modify files or dispatch other agents.",
    },
    Subagent {
        name: "implementer",
        description: "Implements scoped code changes and verifies the result.",
        model: "sonnet",
        prompt: "Implement the requested scoped change. Read the affected code before editing. Change only the files the dispatch prompt names; preserve unrelated work. Run focused verification and report the evidence and remaining limits.",
    },
    Subagent {
        name: "reviser",
        description: "Fixes or finishes work that fell short of the plan, then verifies and reports.",
        model: "opus",
        prompt: "Repair work that fell short of the plan. Compare the result with the plan, fix missing or broken parts yourself, change only what the plan requires or the defect touches, then verify and report what you changed and the evidence.",
    },
];

pub fn build_agents_args() -> Vec<String> {
    let agents: Map<String, Value> = SUBAGENTS
        .iter()
        .map(|agent| {
            (
                agent.name.to_string(),
                json!({
                    "description": agent.description,
                    "prompt": agent.prompt,
                    "model": agent.model,
                }),
            )
        })
        .collect();
    vec!["--agents".to_string(), Value::Object(agents).to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{extract_costs_from_events, parse_stream_line, ParsedLine};

    const BLOCKING_LIMIT_EVENT: &str = r#"{"duration_api_ms":7597,"stop_reason":"stop_sequence","session_id":"27577908-328e-4174-bc2b-9f6fc57be9dc","total_cost_usd":0.17951885,"usage":{"output_tokens_details":{"thinking_tokens":0},"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":0,"ephemeral_5m_input_tokens":0},"inference_geo":"","iterations":[],"speed":"standard","fallback_credit":null},"modelUsage":{"claude-haiku-5-5":{"inputTokens":4,"outputTokens":181,"cacheReadInputTokens":136727,"cacheCreationInputTokens":172228,"webSearchRequests":0,"costUSD":0.17951885,"contextWindow":200000,"maxOutputTokens":128000,"thinkingTokens":171,"canonicalModel":"claude-haiku-5-5","provider":"firstParty","costBasis":"list"}},"permission_denials":[],"terminal_reason":"blocking_limit","fast_mode_state":"off","fast_mode_disabled_reason":"sdk_opt_in_required","subagent_stats":{"spawned":0,"requested":{"background":0,"foreground":0,"unset":0},"started_in_background":0,"max_depth":0,"spawned_by_subagents":0,"completed":0,"failed":0,"killed":{"parent":0,"user":0,"system":0},"refused":{"depth_limit":0,"concurrency_limit":0,"budget":0},"by_type":{}},"safety_stops":0,"is_error":true,"num_turns":1,"subtype":"success","api_error_status":null,"result":"Prompt is too long","type":"result","duration_ms":69,"uuid":"5f4d9442-8652-4f36-8590-63ecdc20db71","queued_turn_count":0,"result_index":2}"#;

    const LIMITS: ResetLimits = ResetLimits {
        turns: RESET_TURNS,
        tokens: RESET_TOKENS,
    };

    fn costs(context_tokens: Option<u64>, terminal_reason: Option<&str>) -> IterationCosts {
        IterationCosts {
            context_tokens,
            terminal_reason: terminal_reason.map(str::to_string),
            ..IterationCosts::default()
        }
    }

    #[test]
    fn parses_terminal_reason_from_the_real_blocking_limit_event() {
        let ParsedLine::Event(event) = parse_stream_line(BLOCKING_LIMIT_EVENT) else {
            panic!("event did not parse");
        };
        let costs = extract_costs_from_events(&[event]).unwrap();
        assert_eq!(costs.terminal_reason.as_deref(), Some("blocking_limit"));
        assert!(hit_blocking_limit(&costs));
    }

    #[test]
    fn blocking_limit_ignores_the_prose() {
        let event = BLOCKING_LIMIT_EVENT.replace(
            "\"terminal_reason\":\"blocking_limit\"",
            "\"terminal_reason\":\"completed\"",
        );
        let ParsedLine::Event(event) = parse_stream_line(&event) else {
            panic!("event did not parse");
        };
        let costs = extract_costs_from_events(&[event]).unwrap();
        assert!(!hit_blocking_limit(&costs));
    }

    #[test]
    fn reset_reason_turn_boundary() {
        assert_eq!(reset_reason(&LIMITS, 29, &costs(None, None)), None);
        for turns in [30, 31] {
            assert_eq!(
                reset_reason(&LIMITS, turns, &costs(None, None)),
                Some(ResetReason::Turns)
            );
        }
    }

    #[test]
    fn reset_reason_token_boundary() {
        assert_eq!(reset_reason(&LIMITS, 1, &costs(Some(139_999), None)), None);
        assert_eq!(
            reset_reason(&LIMITS, 1, &costs(Some(140_000), None)),
            Some(ResetReason::Tokens)
        );
        assert_eq!(reset_reason(&LIMITS, 1, &costs(None, None)), None);
    }

    #[test]
    fn reset_reason_on_blocking_limit_only_by_reason() {
        assert_eq!(
            reset_reason(&LIMITS, 1, &costs(Some(0), Some("blocking_limit"))),
            Some(ResetReason::Tokens)
        );
        assert_eq!(
            reset_reason(&LIMITS, 1, &costs(Some(0), Some("completed"))),
            None
        );
    }

    #[test]
    fn reset_reason_checks_turns_before_tokens() {
        assert_eq!(
            reset_reason(&LIMITS, 30, &costs(Some(200_000), Some("blocking_limit"))),
            Some(ResetReason::Turns)
        );
    }

    #[test]
    fn worker_check_secs_resolution() {
        assert_eq!(resolve_worker_check_secs(None), Some(60));
        assert_eq!(resolve_worker_check_secs(Some(0)), None);
        assert_eq!(resolve_worker_check_secs(Some(15)), Some(15));
    }

    #[test]
    fn watch_timeout_resolution() {
        assert_eq!(resolve_watch_timeout(None), Ok(Duration::from_secs(3600)));
        assert_eq!(resolve_watch_timeout(Some(90)), Ok(Duration::from_secs(90)));
        assert!(resolve_watch_timeout(Some(0))
            .unwrap_err()
            .contains("watch_timeout_secs"));
    }

    #[test]
    fn wrap_width_resolution() {
        assert_eq!(
            resolve_wrap_width(Some(70), Some("90"), Some(100)),
            Ok(Some(70))
        );
        assert_eq!(
            resolve_wrap_width(None, Some(" 90 "), Some(100)),
            Ok(Some(90))
        );
        assert_eq!(resolve_wrap_width(None, None, Some(100)), Ok(Some(100)));
        assert_eq!(resolve_wrap_width(None, None, None), Ok(None));
        assert_eq!(resolve_wrap_width(Some(0), None, Some(100)), Ok(None));
        assert_eq!(resolve_wrap_width(None, Some("0"), Some(100)), Ok(None));
        assert!(resolve_wrap_width(None, Some("wide"), None)
            .unwrap_err()
            .contains("\"wide\""));
    }

    #[test]
    fn recent_history_tokens_resolution() {
        assert_eq!(resolve_recent_history_tokens(Some(5), Some(9)), 5);
        assert_eq!(resolve_recent_history_tokens(None, Some(9)), 9);
        assert_eq!(resolve_recent_history_tokens(None, None), 8192);
        assert_eq!(resolve_recent_history_tokens(Some(0), Some(9)), 0);
        assert_eq!(resolve_recent_history_tokens(None, Some(0)), 0);
    }

    #[test]
    fn turn_system_prompt_appends_older_lines() {
        assert_eq!(turn_system_prompt(&[]), SYSTEM_PROMPT);
        let older = vec!["#0 a".to_string(), "#1 b".to_string()];
        let prompt = turn_system_prompt(&older);
        assert!(prompt.starts_with(SYSTEM_PROMPT));
        assert!(prompt.ends_with("#0 a\n#1 b"));
    }

    #[test]
    fn system_prompt_states_working_rules() {
        for needle in [
            "lead with the answer",
            "one recommended next step",
            "call `suggest_reply` first, before you write any reply text",
            "never mention the suggestion",
            "call `help` first and answer from its text",
            "call `help` with topic `config`, then `config <section>`",
            "asks to change how ralph behaves or looks",
            "propose a code change only when no setting covers it",
            "copy it to config.toml.bak",
            "run `ralph config check`; if it fails, copy config.toml.bak back over config.toml and report the error",
            "3-5 line plan",
            "never state as fact what you have not checked",
            "unverified until you check its diff or test output",
            "name its #<id>",
            "Report not-found, do not widen scope.",
            "After 2 failed repair rounds",
            "you have no erase tool",
            "You are a subagent. Don't run mem.",
        ] {
            assert!(SYSTEM_PROMPT.contains(needle), "{needle}");
        }
    }

    #[test]
    fn system_prompt_requires_approval_before_watch() {
        assert!(SYSTEM_PROMPT.contains(
            "Before calling watch, show the person the exact command and wait for their approval."
        ));
        assert!(SYSTEM_PROMPT.contains("[worker watch:<name> · watch]"));
    }

    #[test]
    fn turn_prompt_puts_view_before_message() {
        let lines = vec!["0 user: hi".to_string(), "#1-2 s".to_string()];
        assert_eq!(
            turn_prompt(&lines, "yo"),
            "0 user: hi\n#1-2 s\n\nNew message:\nyo"
        );
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn select_servers_copies_named_entry_verbatim() {
        let config = json!({"mcpServers": {"a": {"command": "x", "env": {"K": "v"}}, "b": {}}});
        let picked = select_servers(&config, &names(&["a"]));
        assert_eq!(picked.servers.len(), 1);
        assert_eq!(picked.servers["a"], config["mcpServers"]["a"]);
        assert!(picked.missing.is_empty());
    }

    #[test]
    fn select_servers_reports_missing_names() {
        let config = json!({"mcpServers": {"a": {}}});
        let picked = select_servers(&config, &names(&["a", "z"]));
        assert_eq!(picked.missing, names(&["z"]));
    }

    #[test]
    fn select_servers_handles_config_without_mcp_servers() {
        let picked = select_servers(&json!({}), &names(&["a"]));
        assert!(picked.servers.is_empty());
        assert_eq!(picked.missing, names(&["a"]));
    }

    #[test]
    fn select_servers_refuses_ralph_chat_name() {
        let config = json!({"mcpServers": {SERVER_NAME: {"command": "evil"}}});
        let picked = select_servers(&config, &names(&[SERVER_NAME]));
        assert!(picked.servers.is_empty());
        assert_eq!(picked.missing, names(&[SERVER_NAME]));
    }

    #[test]
    fn mcp_config_keeps_ralph_chat_when_user_defines_same_name() {
        let mut imported = Map::new();
        imported.insert(SERVER_NAME.to_string(), json!({"command": "evil"}));
        let config = mcp_config(
            Path::new("/bin/ralph"),
            Path::new("/s"),
            Path::new("/s/chat-1.sock"),
            &imported,
        );
        assert_eq!(config["mcpServers"][SERVER_NAME]["command"], "/bin/ralph");
    }

    #[test]
    fn mcp_config_with_no_imports_matches_old_shape() {
        let config = mcp_config(
            Path::new("/bin/ralph"),
            Path::new("/s"),
            Path::new("/s/chat-1.sock"),
            &Map::new(),
        );
        let servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[SERVER_NAME]["args"],
            json!(["chat-mcp", "--store", "/s", "--socket", "/s/chat-1.sock"])
        );
    }

    #[test]
    fn build_mcp_args_allows_ralph_chat_tools() {
        let config = mcp_config(
            Path::new("/bin/ralph"),
            Path::new("/s"),
            Path::new("/s/chat-1.sock"),
            &Map::new(),
        );
        assert_eq!(
            build_mcp_args(Path::new("/s/mcp.json"), &config),
            [
                "--mcp-config",
                "/s/mcp.json",
                "--allowedTools",
                "mcp__ralph-chat__*",
                "--strict-mcp-config"
            ]
        );
    }

    #[test]
    fn build_mcp_args_allows_every_imported_server() {
        let mut imported = Map::new();
        imported.insert("mcptools".to_string(), json!({ "command": "mcptools" }));
        imported.insert("linear".to_string(), json!({ "command": "linear" }));
        let config = mcp_config(
            Path::new("/bin/ralph"),
            Path::new("/s"),
            Path::new("/s/chat-1.sock"),
            &imported,
        );
        assert_eq!(
            build_mcp_args(Path::new("/s/mcp.json"), &config)[3],
            "mcp__mcptools__*,mcp__linear__*,mcp__ralph-chat__*"
        );
    }

    #[test]
    fn build_nap_args_passes_the_store() {
        assert_eq!(
            build_nap_args(Path::new("/chat store")),
            ["chat-nap", "--store", "/chat store"].map(OsString::from)
        );
    }

    #[test]
    fn build_attachments_args_adds_store_attachments_dir() {
        assert_eq!(
            build_attachments_args(Path::new("/s")),
            ["--add-dir", "/s/attachments"]
        );
    }

    #[test]
    fn build_agents_args_lists_three_roles_with_models() {
        let args = build_agents_args();
        assert_eq!(args[0], "--agents");
        let parsed: Value = serde_json::from_str(&args[1]).unwrap();
        let roles = parsed.as_object().unwrap();
        assert_eq!(roles.len(), 3);
        assert_eq!(parsed["evidence-collector"]["model"], "haiku");
        assert_eq!(parsed["implementer"]["model"], "sonnet");
        assert_eq!(parsed["reviser"]["model"], "opus");
    }
}
