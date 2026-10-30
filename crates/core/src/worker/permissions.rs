use crate::worker::{Report, ReportStatus};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlockedContent {
    pub request_id: String,
    pub tool: String,
    pub summary: String,
    pub options: Vec<PermissionOption>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
}

#[derive(Deserialize)]
struct PublishedRequest {
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    pending: Vec<String>,
}

pub fn pending_requests(marker: &str) -> Vec<String> {
    match serde_json::from_str::<PublishedRequest>(marker) {
        Ok(published) if !published.pending.is_empty() => published.pending,
        Ok(published) if !published.request_id.is_empty() => vec![published.request_id],
        _ => Vec::new(),
    }
}

impl BlockedContent {
    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    #[default]
    Ask,
    Yolo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionAnswer {
    pub request_id: String,
    pub option_id: String,
}

impl PermissionAnswer {
    pub fn decode(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("unreadable permission answer: {e}"))
    }
}

pub fn process_request_id(process: &str, raw: &str) -> String {
    format!("{process}.{raw}")
}

pub fn pending_position<'a>(
    pending: impl Iterator<Item = &'a str>,
    request_id: &str,
) -> Result<usize, String> {
    let request_id = request_id.trim();
    let ids: Vec<&str> = pending.collect();
    ids.iter()
        .position(|id| !request_id.is_empty() && *id == request_id)
        .ok_or_else(|| {
            let listed = if ids.is_empty() {
                "none".to_string()
            } else {
                ids.join(", ")
            };
            if request_id.is_empty() {
                format!(
                    "this worker is waiting on a permission request; pass request. pending: {listed}"
                )
            } else {
                format!("stale request {request_id}; pending: {listed}")
            }
        })
}

pub fn match_option<'a>(
    options: &'a [PermissionOption],
    text: &str,
) -> Option<&'a PermissionOption> {
    let text = text.trim();
    options.iter().find(|option| option.option_id == text)
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
            "Worker {id} is waiting for a permission answer on {}: {}. Answer it with reply, request {} and one optionId: {}.",
            content.tool,
            content.summary,
            content.request_id,
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
    fn match_option_takes_only_an_exact_option_id() {
        let options = options();
        assert_eq!(match_option(&options, "allow-once"), Some(&options[0]));
        assert_eq!(match_option(&options, " reject-once\n"), Some(&options[1]));
        assert_eq!(match_option(&options, &options[0].name), None);
        assert_eq!(match_option(&options, "Allow-Once"), None);
        assert_eq!(match_option(&options, "yes"), None);
        assert_eq!(match_option(&options, ""), None);
    }

    #[test]
    fn the_permission_alert_names_the_tool_and_every_option() {
        let content = BlockedContent {
            request_id: "7".into(),
            tool: "shell".into(),
            summary: "ls".into(),
            options: options(),
            pending: Vec::new(),
        };
        let report = permission_alert_report("A", "t", &content);
        assert!(report.text.contains("shell"));
        assert!(report.text.contains("request 7"));
        assert!(report.text.contains("allow-once, reject-once"));
    }

    #[test]
    fn pending_requests_keep_queue_order_and_fall_back_to_the_front() {
        assert_eq!(
            pending_requests(r#"{"request_id":"b","pending":["b","a"]}"#),
            ["b", "a"]
        );
        assert_eq!(pending_requests(r#"{"request_id":"b","tool":"t"}"#), ["b"]);
        assert!(pending_requests("").is_empty());
    }

    #[test]
    fn an_answer_without_a_request_id_lists_every_pending_id() {
        assert_eq!(
            pending_position(["a", "b"].into_iter(), " "),
            Err(
                "this worker is waiting on a permission request; pass request. pending: a, b"
                    .into()
            )
        );
    }

    #[test]
    fn a_request_id_carries_its_host_process_before_the_raw_id() {
        assert_eq!(process_request_id("12-34", "0"), "12-34.0");
        assert_ne!(process_request_id("a", "0"), process_request_id("b", "0"));
    }

    #[test]
    fn no_option_message_lists_every_option_id() {
        let message = no_option_message(&options());
        assert!(message.contains("allow-once, reject-once"));
    }
}
