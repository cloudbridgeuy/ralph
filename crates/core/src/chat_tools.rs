use crate::chat::MCPTOOLS_SERVER;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolRule {
    Server(String),
    Tool { server: String, tool: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPolicy {
    pub allow: Vec<ToolRule>,
    pub ask: Vec<ToolRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolVerdict {
    Allow,
    Ask,
}

impl ToolRule {
    pub fn claude(&self) -> String {
        match self {
            ToolRule::Server(server) => format!("mcp__{server}__*"),
            ToolRule::Tool { server, tool } => format!("mcp__{server}__{tool}"),
        }
    }

    pub fn matches(&self, claude_name: &str) -> bool {
        match self {
            ToolRule::Server(server) => claude_name.starts_with(&format!("mcp__{server}__")),
            ToolRule::Tool { .. } => claude_name == self.claude(),
        }
    }

    pub fn allow_for(server: &str) -> ToolRule {
        if server == MCPTOOLS_SERVER {
            ToolRule::mcptools("find_tools")
        } else {
            ToolRule::Server(server.to_string())
        }
    }

    fn mcptools(tool: &str) -> ToolRule {
        ToolRule::Tool {
            server: MCPTOOLS_SERVER.to_string(),
            tool: tool.to_string(),
        }
    }
}

pub fn mcptools_ask_rules() -> Vec<ToolRule> {
    vec![
        ToolRule::mcptools("execute"),
        ToolRule::mcptools("call_tool"),
    ]
}

pub fn chat_tool_policy<'a>(servers: impl IntoIterator<Item = &'a str>) -> ToolPolicy {
    ToolPolicy {
        allow: servers.into_iter().map(ToolRule::allow_for).collect(),
        ask: mcptools_ask_rules(),
    }
}

pub fn tool_verdict(policy: &ToolPolicy, claude_name: &str) -> ToolVerdict {
    let matches = |rules: &[ToolRule]| rules.iter().any(|rule| rule.matches(claude_name));
    if !matches(&policy.ask) && matches(&policy.allow) {
        ToolVerdict::Allow
    } else {
        ToolVerdict::Ask
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ToolPolicy {
        chat_tool_policy(["ralph-chat", "mcptools", "linear"])
    }

    #[test]
    fn tool_verdicts_follow_allow_and_ask_rules() {
        let verdict = |name| tool_verdict(&policy(), name);
        assert_eq!(verdict("mcp__ralph-chat__recall"), ToolVerdict::Allow);
        assert_eq!(verdict("mcp__linear__get_issue"), ToolVerdict::Allow);
        assert_eq!(verdict("mcp__mcptools__find_tools"), ToolVerdict::Allow);
        assert_eq!(verdict("mcp__mcptools__execute"), ToolVerdict::Ask);
        assert_eq!(verdict("mcp__mcptools__call_tool"), ToolVerdict::Ask);
        assert_eq!(verdict("run_command"), ToolVerdict::Ask);
        assert_eq!(verdict("mcp__other__x"), ToolVerdict::Ask);
    }

    #[test]
    fn tool_ask_rules_win_over_server_wildcards() {
        let policy = ToolPolicy {
            allow: vec![ToolRule::Server("mcptools".into())],
            ask: mcptools_ask_rules(),
        };
        assert_eq!(
            tool_verdict(&policy, "mcp__mcptools__execute"),
            ToolVerdict::Ask
        );
        assert_eq!(
            tool_verdict(&policy, "mcp__mcptools__other"),
            ToolVerdict::Allow
        );
    }

    #[test]
    fn tool_policy_matches_claude_allowed_tools() {
        let allow: Vec<String> = policy().allow.iter().map(ToolRule::claude).collect();
        assert_eq!(
            allow.join(","),
            "mcp__ralph-chat__*,mcp__mcptools__find_tools,mcp__linear__*"
        );
    }
}
