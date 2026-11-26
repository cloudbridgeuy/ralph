use ralph_core::chat_tools::{tool_verdict, ToolPolicy, ToolVerdict};
use ralph_core::worker::{
    claude_tool_name, permission_content, refusal, ClaudePermission, PermissionOption,
};
use serde_json::Value;

#[allow(dead_code, reason = "the chat session constructs and answers these")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionPolicy {
    RejectAll,
    Chat(ToolPolicy),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Select(String),
    Ask(ClaudePermission),
}

fn option_of_kind<'a>(options: &'a [PermissionOption], kind: &str) -> Option<&'a PermissionOption> {
    options.iter().find(|option| option.kind == kind)
}

pub fn decide(policy: &PermissionPolicy, request: &Value) -> Option<Decision> {
    let content = permission_content(request);
    match policy {
        PermissionPolicy::RejectAll => refusal(&content)
            .option_id()
            .map(|option| Decision::Select(option.to_string())),
        PermissionPolicy::Chat(tools) => {
            let allowed =
                tool_verdict(tools, &claude_tool_name(&content.tool)) == ToolVerdict::Allow;
            let allow_once = option_of_kind(&content.options, "allow_once").filter(|_| allowed);
            Some(match allow_once {
                Some(option) => Decision::Select(option.option_id.clone()),
                None => Decision::Ask(ClaudePermission {
                    input: request["toolCall"]["rawInput"].clone(),
                    content,
                }),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_tools::chat_tool_policy;
    use serde_json::{json, Value};

    fn request(name: &str, kinds: &[&str]) -> Value {
        let options: Vec<Value> = kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| json!({"optionId": format!("o{index}"), "name": *kind, "kind": *kind}))
            .collect();
        json!({"sessionId": "s", "toolCall": {"toolCallId": "c", "name": name, "title": "do it", "rawInput": {"command": "ls"}}, "options": options})
    }

    fn chat() -> PermissionPolicy {
        PermissionPolicy::Chat(chat_tool_policy(["ralph-chat"]))
    }

    #[test]
    fn decide_reject_all_selects_reject_option() {
        let chosen = decide(
            &PermissionPolicy::RejectAll,
            &request("run_command", &["allow_once", "reject_once"]),
        );
        assert_eq!(chosen, Some(Decision::Select("o1".to_string())));
        let none = decide(&PermissionPolicy::RejectAll, &request("x", &["allow_once"]));
        assert_eq!(none, None);
    }

    #[test]
    fn decide_chat_allows_chat_mcp_tool() {
        let chosen = decide(
            &chat(),
            &request("mcp_ralph-chat_report", &["reject_once", "allow_once"]),
        );
        assert_eq!(chosen, Some(Decision::Select("o1".to_string())));
    }

    #[test]
    fn decide_chat_asks_for_run_command() {
        let Some(Decision::Ask(ask)) = decide(
            &chat(),
            &request("run_command", &["allow_once", "reject_once"]),
        ) else {
            panic!("expected Ask")
        };
        assert_eq!(ask.content.tool, "run_command");
        assert_eq!(ask.content.summary, "do it");
        assert_eq!(ask.input, json!({"command": "ls"}));
    }

    #[test]
    fn decide_chat_asks_when_no_allow_option() {
        let chosen = decide(&chat(), &request("mcp_ralph-chat_report", &["reject_once"]));
        assert!(matches!(chosen, Some(Decision::Ask(_))));
    }
}
