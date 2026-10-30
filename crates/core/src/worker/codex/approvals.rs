use crate::worker::{BlockedContent, PermissionOption};
use serde_json::{json, Value};

pub const COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
pub const FILE_CHANGE_APPROVAL: &str = "item/fileChange/requestApproval";
pub const ELICITATION_REQUEST: &str = "mcpServer/elicitation/request";
pub const PERMISSIONS_APPROVAL: &str = "item/permissions/requestApproval";
pub const SERVER_REQUEST_RESOLVED: &str = "serverRequest/resolved";

const ACCEPT: &str = "accept";
const DECLINE: &str = "decline";
const CANCEL: &str = "cancel";
const ACCEPT_FOR_SESSION: &str = "acceptForSession";
const MCP_TOOL_CALL: &str = "mcp_tool_call";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalKind {
    Command,
    FileChange,
    McpToolCall,
    Elicitation,
    PermissionsGrant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval {
    pub kind: ApprovalKind,
    pub content: BlockedContent,
}

fn option(id: &str) -> PermissionOption {
    let (name, kind) = match id {
        ACCEPT => ("Accept", "allow_once"),
        DECLINE => ("Decline", "reject_once"),
        CANCEL => ("Cancel", "reject_once"),
        other => (other, "other"),
    };
    PermissionOption {
        option_id: id.to_string(),
        name: name.to_string(),
        kind: kind.to_string(),
    }
}

fn options(ids: &[&str]) -> Vec<PermissionOption> {
    ids.iter().map(|id| option(id)).collect()
}

fn is_published(id: &str) -> bool {
    id != ACCEPT_FOR_SESSION && !id.contains("Amendment")
}

fn command_options(params: &Value) -> Vec<PermissionOption> {
    match params["availableDecisions"].as_array() {
        Some(decisions) => decisions
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| is_published(id))
            .map(option)
            .collect(),
        None => options(&[ACCEPT, DECLINE, CANCEL]),
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params[key].as_str().filter(|text| !text.trim().is_empty())
}

fn approval(
    kind: ApprovalKind,
    tool: &str,
    summary: &str,
    options: Vec<PermissionOption>,
) -> Approval {
    Approval {
        kind,
        content: BlockedContent {
            request_id: String::new(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            options,
            pending: Vec::new(),
        },
    }
}

pub fn classify(method: &str, params: &Value) -> Option<Approval> {
    match method {
        COMMAND_APPROVAL => Some(approval(
            ApprovalKind::Command,
            "command",
            text(params, "command")
                .or_else(|| text(params, "reason"))
                .unwrap_or("command execution"),
            command_options(params),
        )),
        FILE_CHANGE_APPROVAL => Some(approval(
            ApprovalKind::FileChange,
            "file change",
            text(params, "reason").unwrap_or("file change"),
            options(&[ACCEPT, DECLINE, CANCEL]),
        )),
        ELICITATION_REQUEST => {
            let tool_call = params["_meta"]["codex_approval_kind"] == MCP_TOOL_CALL;
            let (kind, ids): (_, &[&str]) = if tool_call {
                (ApprovalKind::McpToolCall, &[ACCEPT, DECLINE, CANCEL])
            } else {
                (ApprovalKind::Elicitation, &[DECLINE, CANCEL])
            };
            Some(approval(
                kind,
                text(params, "serverName").unwrap_or("mcp server"),
                text(params, "message").unwrap_or("elicitation"),
                options(ids),
            ))
        }
        PERMISSIONS_APPROVAL => Some(approval(
            ApprovalKind::PermissionsGrant,
            "permissions",
            text(params, "reason").unwrap_or("extra permissions"),
            options(&[DECLINE]),
        )),
        _ => None,
    }
}

pub fn yolo_option(approval: &Approval) -> Option<&PermissionOption> {
    matches!(
        approval.kind,
        ApprovalKind::Command | ApprovalKind::FileChange | ApprovalKind::McpToolCall
    )
    .then(|| {
        approval
            .content
            .options
            .iter()
            .find(|option| option.option_id == ACCEPT)
    })
    .flatten()
}

pub fn response_body(kind: ApprovalKind, option_id: &str) -> Value {
    match kind {
        ApprovalKind::Command | ApprovalKind::FileChange => json!({"decision": option_id}),
        ApprovalKind::McpToolCall | ApprovalKind::Elicitation => json!({"action": option_id}),
        ApprovalKind::PermissionsGrant => json!({"permissions": {}, "scope": "turn"}),
    }
}

pub fn resolved_request_id(method: &str, params: &Value) -> Option<Value> {
    (method == SERVER_REQUEST_RESOLVED)
        .then(|| params.get("requestId").cloned())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(approval: &Approval) -> Vec<&str> {
        approval
            .content
            .options
            .iter()
            .map(|option| option.option_id.as_str())
            .collect()
    }

    fn command(decisions: Value) -> Value {
        json!({"threadId": "t", "turnId": "u", "itemId": "i", "command": "curl x", "availableDecisions": decisions})
    }

    #[test]
    fn a_command_publishes_its_decisions_without_amendments_or_session_grants() {
        let params = command(json!([
            "accept",
            "acceptForSession",
            {"acceptWithExecpolicyAmendment": {"execpolicy_amendment": ["curl"]}},
            {"applyNetworkPolicyAmendment": {"network_policy_amendment": {}}},
            "cancel"
        ]));
        let approval = classify(COMMAND_APPROVAL, &params).unwrap();
        assert_eq!(approval.kind, ApprovalKind::Command);
        assert_eq!(approval.content.summary, "curl x");
        assert_eq!(ids(&approval), ["accept", "cancel"]);
    }

    #[test]
    fn a_command_without_a_decision_list_offers_accept_decline_cancel() {
        let approval = classify(COMMAND_APPROVAL, &json!({"itemId": "i"})).unwrap();
        assert_eq!(ids(&approval), ["accept", "decline", "cancel"]);
    }

    #[test]
    fn file_changes_and_tool_call_elicitations_offer_only_single_request_options() {
        let file = classify(FILE_CHANGE_APPROVAL, &json!({"grantRoot": "/"})).unwrap();
        assert_eq!(file.kind, ApprovalKind::FileChange);
        assert_eq!(ids(&file), ["accept", "decline", "cancel"]);
        let params = json!({"serverName": "m", "message": "run it", "mode": "form", "_meta": {"codex_approval_kind": "mcp_tool_call", "persist": ["session", "always"]}});
        let tool = classify(ELICITATION_REQUEST, &params).unwrap();
        assert_eq!(tool.kind, ApprovalKind::McpToolCall);
        assert_eq!(ids(&tool), ["accept", "decline", "cancel"]);
    }

    #[test]
    fn a_user_question_elicitation_cannot_be_accepted() {
        let params = json!({"serverName": "m", "message": "name?", "mode": "form"});
        let question = classify(ELICITATION_REQUEST, &params).unwrap();
        assert_eq!(question.kind, ApprovalKind::Elicitation);
        assert_eq!(ids(&question), ["decline", "cancel"]);
        assert!(yolo_option(&question).is_none());
    }

    #[test]
    fn yolo_accepts_a_command_only_when_accept_is_offered() {
        let offered = classify(COMMAND_APPROVAL, &command(json!(["accept", "cancel"]))).unwrap();
        assert_eq!(
            yolo_option(&offered).map(|o| o.option_id.as_str()),
            Some("accept")
        );
        let absent = classify(
            COMMAND_APPROVAL,
            &command(json!(["acceptForSession", "cancel"])),
        )
        .unwrap();
        assert!(yolo_option(&absent).is_none());
        let amendment_only = classify(
            COMMAND_APPROVAL,
            &command(json!([{"acceptWithExecpolicyAmendment": {"execpolicy_amendment": []}}])),
        )
        .unwrap();
        assert!(yolo_option(&amendment_only).is_none());
    }

    #[test]
    fn yolo_accepts_file_changes_and_tool_call_elicitations() {
        let file = classify(FILE_CHANGE_APPROVAL, &json!({})).unwrap();
        assert!(yolo_option(&file).is_some());
        let tool = classify(
            ELICITATION_REQUEST,
            &json!({"_meta": {"codex_approval_kind": "mcp_tool_call"}}),
        )
        .unwrap();
        assert!(yolo_option(&tool).is_some());
    }

    #[test]
    fn a_permissions_grant_stays_pending_in_yolo_and_grants_nothing() {
        let grant = classify(PERMISSIONS_APPROVAL, &json!({"reason": "net"})).unwrap();
        assert_eq!(grant.kind, ApprovalKind::PermissionsGrant);
        assert!(yolo_option(&grant).is_none());
        assert_eq!(ids(&grant), ["decline"]);
        assert_eq!(
            response_body(grant.kind, "decline"),
            json!({"permissions": {}, "scope": "turn"})
        );
    }

    #[test]
    fn unknown_requests_are_not_approvals() {
        assert!(classify("item/tool/requestUserInput", &json!({})).is_none());
        assert!(classify("item/tool/call", &json!({})).is_none());
    }

    #[test]
    fn response_bodies_carry_no_persist_or_session_scope() {
        assert_eq!(
            response_body(ApprovalKind::Command, "accept"),
            json!({"decision": "accept"})
        );
        assert_eq!(
            response_body(ApprovalKind::FileChange, "cancel"),
            json!({"decision": "cancel"})
        );
        let elicitation = response_body(ApprovalKind::McpToolCall, "accept");
        assert_eq!(elicitation, json!({"action": "accept"}));
        assert!(elicitation.get("_meta").is_none() && elicitation.get("content").is_none());
    }

    #[test]
    fn only_a_resolved_notification_yields_a_request_id() {
        let params = json!({"threadId": "t", "requestId": 7});
        assert_eq!(
            resolved_request_id(SERVER_REQUEST_RESOLVED, &params),
            Some(json!(7))
        );
        assert_eq!(resolved_request_id("turn/started", &params), None);
    }
}
