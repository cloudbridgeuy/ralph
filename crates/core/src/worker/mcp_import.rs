use super::SERVER_NAME;
use crate::chat::mcp_wildcards;
use serde_json::{json, Map, Value};

pub fn merged_mcp_config(imported: &Map<String, Value>, own: &Value) -> Value {
    let mut servers = imported.clone();
    for (name, server) in own["mcpServers"].as_object().into_iter().flatten() {
        servers.insert(name.clone(), server.clone());
    }
    json!({ "mcpServers": servers })
}

pub fn missing_server_warning(name: &str) -> String {
    format!("worker: MCP server \"{name}\" not found in ~/.claude.json; continuing without it")
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
}
