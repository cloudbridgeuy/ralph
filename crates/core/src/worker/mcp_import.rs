use super::SERVER_NAME;
use crate::chat::{mcp_wildcards, select_servers};
use crate::chat_memory::SERVER_NAME as CHAT_SERVER_NAME;
use serde_json::{json, Map, Value};

pub fn merged_mcp_config(imported: &Map<String, Value>, own: &Value) -> Value {
    let mut servers = imported.clone();
    for (name, server) in own["mcpServers"].as_object().into_iter().flatten() {
        servers.insert(name.clone(), server.clone());
    }
    json!({ "mcpServers": servers })
}

pub struct WorkerImport {
    pub servers: Map<String, Value>,
    pub warnings: Vec<String>,
}

pub fn import_servers(claude_config: &Value, names: &[String]) -> WorkerImport {
    let (reserved, rest): (Vec<&String>, Vec<&String>) = names
        .iter()
        .partition(|name| *name == SERVER_NAME || *name == CHAT_SERVER_NAME);
    let rest: Vec<String> = rest.into_iter().cloned().collect();
    let imported = select_servers(claude_config, &rest);
    WorkerImport {
        servers: imported.servers,
        warnings: reserved
            .iter()
            .map(|name| reserved_server_warning(name))
            .chain(
                imported
                    .missing
                    .iter()
                    .map(|name| missing_server_warning(name)),
            )
            .collect(),
    }
}

fn reserved_server_warning(name: &str) -> String {
    format!("worker: MCP server \"{name}\" is reserved by ralph; continuing without it")
}

fn missing_server_warning(name: &str) -> String {
    format!("worker: MCP server \"{name}\" not found in ~/.claude.json; continuing without it")
}

pub fn join_start_warnings(warnings: &[String]) -> Option<String> {
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

pub fn allowed_tools(copied: &[String]) -> String {
    std::iter::once(own_tool_names())
        .chain(mcp_wildcards(copied))
        .collect::<Vec<_>>()
        .join(",")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_tools_are_the_own_tools_plus_one_wildcard_per_copied_server() {
        assert_eq!(
            allowed_tools(&["x".to_string()]),
            "mcp__ralph-worker__report,mcp__ralph-worker__set_status,mcp__x__*"
        );
        assert_eq!(
            allowed_tools(&[]),
            "mcp__ralph-worker__report,mcp__ralph-worker__set_status"
        );
    }

    #[test]
    fn the_merged_config_holds_imports_and_the_own_server() {
        let mut imported = Map::new();
        imported.insert("mcptools".into(), json!({"command": "m"}));
        let own = json!({"mcpServers": {"ralph-worker": {"command": "r"}}});
        let merged = merged_mcp_config(&imported, &own);
        assert_eq!(merged["mcpServers"]["mcptools"], json!({"command": "m"}));
        assert_eq!(
            merged["mcpServers"]["ralph-worker"],
            json!({"command": "r"})
        );
    }

    #[test]
    fn the_own_server_wins_a_name_clash() {
        let mut imported = Map::new();
        imported.insert("ralph-worker".into(), json!({"command": "evil"}));
        let own = json!({"mcpServers": {"ralph-worker": {"command": "r"}}});
        let merged = merged_mcp_config(&imported, &own);
        assert_eq!(
            merged["mcpServers"]["ralph-worker"],
            json!({"command": "r"})
        );
    }

    #[test]
    fn the_missing_server_warning_names_the_server() {
        assert_eq!(
            missing_server_warning("docs"),
            "worker: MCP server \"docs\" not found in ~/.claude.json; continuing without it"
        );
    }

    #[test]
    fn start_warnings_are_all_kept_in_order() {
        let warnings = vec!["first".to_string(), "second".to_string()];
        assert_eq!(
            join_start_warnings(&warnings),
            Some("first; second".to_string())
        );
        assert_eq!(
            join_start_warnings(&["only".to_string()]),
            Some("only".into())
        );
        assert_eq!(join_start_warnings(&[]), None);
    }

    fn claude_config() -> Value {
        json!({"mcpServers": {
            "docs": {"command": "d"},
            "ralph-worker": {"command": "evil"},
            "ralph-chat": {"command": "evil"}
        }})
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_worker_own_name_is_never_imported() {
        let imported = import_servers(&claude_config(), &names(&["docs", "ralph-worker"]));
        assert_eq!(
            imported.servers.keys().collect::<Vec<_>>(),
            vec![&"docs".to_string()]
        );
        assert_eq!(
            imported.warnings,
            ["worker: MCP server \"ralph-worker\" is reserved by ralph; continuing without it"]
        );
    }

    #[test]
    fn the_chat_own_name_is_reserved_not_missing() {
        let imported = import_servers(&claude_config(), &names(&["ralph-chat"]));
        assert!(imported.servers.is_empty());
        assert_eq!(
            imported.warnings,
            ["worker: MCP server \"ralph-chat\" is reserved by ralph; continuing without it"]
        );
    }

    #[test]
    fn an_absent_name_still_warns_as_not_found() {
        let imported = import_servers(&claude_config(), &names(&["nope"]));
        assert_eq!(imported.warnings, [missing_server_warning("nope")]);
    }
}
