use crate::chat_memory::SERVER_NAME;
use serde_json::{json, Map, Value};
use std::path::Path;

pub const VIEW_LINES: usize = 200;
pub const FROZEN_WINDOW: usize = 32;

pub fn resolve_frozen_window(flag: Option<usize>, config: Option<usize>) -> Result<usize, String> {
    let window = flag.or(config).unwrap_or(FROZEN_WINDOW);
    if (1..VIEW_LINES).contains(&window) {
        Ok(window)
    } else {
        Err(format!(
            "frozen window {window} is out of range: use a value from 1 to {}.",
            VIEW_LINES - 1
        ))
    }
}

pub const SYSTEM_PROMPT: &str = "\
You are in a long-running chat with one person. The chat so far is compressed, oldest first. The older lines are at the end of this system prompt. The most recent lines start the message you receive, then the new message.

Lines that start with #<id> are summaries of older parts of the chat. The zoom tool opens one into its two halves. The recall tool searches the full chat by regex.

Before you guess or ask the person for a fact, use zoom and recall until the facts are whole.

Messages that start with [worker <id> · <status>] come from agents working for the person, not from the person. Reply to a worker yourself only when the answer is already settled in the chat: a decision the person made, scope they approved, or a step in a plan they agreed. Merges, pushes, PRs, Linear status changes, new scope, and anything public always go to the person: tell them what the worker did or needs, with your recommendation, and wait. List every reply you sent to a worker in your next message to the person. Use spawn_worker and close_worker only to carry out what the person asked in their latest message; set abandon on close_worker only when the person said to drop the work.

Reply format: lead with the answer, in 3 lines if it fits. Flag each risk in one line. End with one recommended next step, not a list of options.

Ask first: when two readings of a request lead to different work, show a 3-5 line plan and wait for a go.

Verify: never state as fact what you have not checked. A worker or subagent \"done\" is unverified until you check its diff or test output, and the git and Linear state.

Cite history: when you state a fact from older chat, check it with zoom or recall, name its #<id>, and say if it came from a summary.

Development roles: you never do the work yourself; you scope it, decide, and accept results. Read inline only to scope, with at most 3 tool calls per question. Reads across several files, broad searches, and test runs go to the evidence-collector subagent. All code edits go to the implementer subagent. Fixing or finishing work that fell short goes to the reviser subagent. Every dispatch prompt states the goal, the exact question, known facts, the files the worker owns (all other code is read-only), output format and size cap, stop condition and tool budget, \"Report not-found, do not widen scope.\", and \"You are a subagent. Don't run mem.\" After 2 failed repair rounds on one issue, diagnose before you dispatch again; after a 3rd, stop and wait for the person.

Forget requests: you have no erase tool. Say so plainly; never summarize the item as resolved.

The chat is logged automatically. Never claim to save, store, or remember anything manually.";

pub fn turn_system_prompt(frozen: &[String]) -> String {
    if frozen.is_empty() {
        return SYSTEM_PROMPT.to_string();
    }
    format!(
        "{SYSTEM_PROMPT}\n\nOlder chat, compressed, oldest first:\n{}",
        frozen.join("\n")
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

pub fn mcp_config(
    exe: &Path,
    store: &Path,
    permission_mode: &str,
    imported: &Map<String, Value>,
) -> Value {
    let mut servers = imported.clone();
    servers.insert(
        SERVER_NAME.to_string(),
        json!({
            "command": exe.to_string_lossy(),
            "args": ["chat-mcp", "--store", store.to_string_lossy(), "--permission-mode", permission_mode]
        }),
    );
    json!({ "mcpServers": servers })
}

pub fn build_mcp_args(config_path: &Path) -> Vec<String> {
    vec![
        "--mcp-config".to_string(),
        config_path.to_string_lossy().into_owned(),
        "--strict-mcp-config".to_string(),
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

    #[test]
    fn frozen_window_resolution() {
        assert_eq!(resolve_frozen_window(Some(5), Some(9)), Ok(5));
        assert_eq!(resolve_frozen_window(None, Some(9)), Ok(9));
        assert_eq!(resolve_frozen_window(None, None), Ok(32));
        assert_eq!(resolve_frozen_window(Some(199), None), Ok(199));
        assert!(resolve_frozen_window(Some(0), None)
            .unwrap_err()
            .contains("1 to 199"));
        assert!(resolve_frozen_window(None, Some(200))
            .unwrap_err()
            .contains("200"));
    }

    #[test]
    fn turn_system_prompt_appends_frozen_lines() {
        assert_eq!(turn_system_prompt(&[]), SYSTEM_PROMPT);
        let frozen = vec!["#0 a".to_string(), "#1 b".to_string()];
        let prompt = turn_system_prompt(&frozen);
        assert!(prompt.starts_with(SYSTEM_PROMPT));
        assert!(prompt.ends_with("#0 a\n#1 b"));
    }

    #[test]
    fn system_prompt_states_working_rules() {
        for needle in [
            "lead with the answer",
            "one recommended next step",
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
        let config = mcp_config(Path::new("/bin/ralph"), Path::new("/s"), "auto", &imported);
        assert_eq!(config["mcpServers"][SERVER_NAME]["command"], "/bin/ralph");
    }

    #[test]
    fn mcp_config_with_no_imports_matches_old_shape() {
        let config = mcp_config(
            Path::new("/bin/ralph"),
            Path::new("/s"),
            "auto",
            &Map::new(),
        );
        let servers = config["mcpServers"].as_object().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[SERVER_NAME]["args"],
            json!(["chat-mcp", "--store", "/s", "--permission-mode", "auto"])
        );
    }

    #[test]
    fn build_mcp_args_passes_path_and_strict_flag() {
        assert_eq!(
            build_mcp_args(Path::new("/s/mcp.json")),
            ["--mcp-config", "/s/mcp.json", "--strict-mcp-config"]
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
