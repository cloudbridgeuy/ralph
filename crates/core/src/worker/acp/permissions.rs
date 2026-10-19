use super::REPORT_TOOL;
use crate::worker::{Report, ReportStatus, SERVER_NAME};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

fn report_tool_label() -> String {
    format!("{SERVER_NAME}-{REPORT_TOOL}: {REPORT_TOOL}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorPermissions {
    #[default]
    Ask,
    Yolo,
}

pub fn auto_allow_option(
    mode: CursorPermissions,
    content: &BlockedContent,
) -> Option<&PermissionOption> {
    let label = report_tool_label();
    let is_report = content.tool.trim() == label || content.summary.trim() == label;
    (mode == CursorPermissions::Yolo || is_report)
        .then(|| {
            content
                .options
                .iter()
                .find(|option| option.kind == "allow_once")
        })
        .flatten()
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

pub fn permission_alert_report(id: &str, at: &str, content: &BlockedContent) -> Report {
    let ids: Vec<&str> = content
        .options
        .iter()
        .map(|option| option.option_id.as_str())
        .collect();
    Report {
        worker: id.to_string(),
        at: at.to_string(),
        status: ReportStatus::Blocked.as_str().to_string(),
        text: format!(
            "Worker {id} is waiting for a permission answer on {}: {}. Answer it with reply and one optionId: {}.",
            content.tool,
            content.summary,
            ids.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            auto_allow_option(ASK, &report).map(|o| o.option_id.as_str()),
            Some("allow-once")
        );
        let by_tool = report_content("ralph-worker-report: report", "x");
        assert!(auto_allow_option(ASK, &by_tool).is_some());
        assert!(auto_allow_option(ASK, &report_content("shell", "Run ls")).is_none());
        assert!(
            auto_allow_option(ASK, &report_content("x", "other-server-report: report")).is_none()
        );
        assert!(auto_allow_option(ASK, &report_content("x", "ralph-chat-reply: reply")).is_none());
    }

    const ASK: CursorPermissions = CursorPermissions::Ask;
    const YOLO: CursorPermissions = CursorPermissions::Yolo;

    #[test]
    fn yolo_allows_any_request_with_allow_once_only() {
        let shell = report_content("shell", "Run ls");
        assert_eq!(
            auto_allow_option(YOLO, &shell).map(|o| o.option_id.as_str()),
            Some("allow-once")
        );
    }

    #[test]
    fn yolo_without_allow_once_waits_and_never_picks_allow_always() {
        let mut content = report_content("shell", "Run ls");
        content.options.push(PermissionOption {
            option_id: "allow-always".into(),
            name: "Always".into(),
            kind: "allow_always".into(),
        });
        content.options.retain(|o| o.kind != "allow_once");
        assert!(auto_allow_option(YOLO, &content).is_none());
    }

    #[test]
    fn report_without_an_allow_once_option_still_waits() {
        let mut content = report_content("x", "ralph-worker-report: report");
        content.options.retain(|o| o.kind != "allow_once");
        assert!(auto_allow_option(ASK, &content).is_none());
    }

    #[test]
    fn the_permission_alert_names_the_tool_and_every_option() {
        let content = BlockedContent {
            tool: "shell".into(),
            summary: "ls".into(),
            options: options(),
        };
        let report = permission_alert_report("A", "t", &content);
        assert!(report.text.contains("shell"));
        assert!(report.text.contains("allow-once, reject-once"));
    }

    #[test]
    fn no_option_message_lists_every_option_id() {
        let message = no_option_message(&options());
        assert!(message.contains("allow-once, reject-once"));
    }
}
