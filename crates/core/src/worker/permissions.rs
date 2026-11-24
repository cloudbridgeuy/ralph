use crate::worker::{ClaudePermission, Report, ReportStatus};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedContent {
    pub request_id: String,
    pub tool: String,
    pub summary: String,
    pub options: Vec<PermissionOption>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum McptoolsDecision {
    Read,
    Elevated(Classes),
    Deny,
    Other,
}

pub fn mcptools_decision(ask: &ClaudePermission) -> McptoolsDecision {
    if !matches!(
        ask.content.tool.as_str(),
        "mcp__mcptools__execute" | "mcp__mcptools__call_tool"
    ) {
        return McptoolsDecision::Other;
    }
    let flags = [ask.input.get("allowWrites"), ask.input.get("allowSpend")];
    if flags
        .iter()
        .any(|flag| !matches!(flag, None | Some(serde_json::Value::Bool(_))))
    {
        return McptoolsDecision::Deny;
    }
    let set =
        |flag: Option<&serde_json::Value>| matches!(flag, Some(serde_json::Value::Bool(true)));
    let classes = Classes {
        writes: set(flags[0]),
        spend: set(flags[1]),
    };
    if classes == Classes::default() {
        McptoolsDecision::Read
    } else {
        McptoolsDecision::Elevated(classes)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Classes {
    pub writes: bool,
    pub spend: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grants {
    #[serde(default)]
    pub writes: bool,
    #[serde(default)]
    pub spend: bool,
}

impl Grants {
    pub fn for_mode(mode: PermissionMode) -> Grants {
        let granted = mode == PermissionMode::Yolo;
        Grants {
            writes: granted,
            spend: granted,
        }
    }

    fn missing(self, needed: Classes) -> Classes {
        Classes {
            writes: needed.writes && !self.writes,
            spend: needed.spend && !self.spend,
        }
    }

    pub fn union(self, other: Grants) -> Grants {
        Grants {
            writes: self.writes || other.writes,
            spend: self.spend || other.spend,
        }
    }
}

pub fn parse_grants(text: Option<&str>) -> (Grants, Option<String>) {
    match text.map(serde_json::from_str::<Grants>) {
        None => (Grants::default(), None),
        Some(Ok(grants)) => (grants, None),
        Some(Err(e)) => (
            Grants::default(),
            Some(format!("unreadable mcptools grants: {e}; no saved grants")),
        ),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum GateCall {
    Allow,
    Deny,
    Prompt(Classes),
    Other,
}

pub fn mcptools_gate(grants: Grants, ask: &ClaudePermission) -> GateCall {
    match mcptools_decision(ask) {
        McptoolsDecision::Other => GateCall::Other,
        McptoolsDecision::Deny => GateCall::Deny,
        McptoolsDecision::Read => GateCall::Allow,
        McptoolsDecision::Elevated(needed) => match grants.missing(needed) {
            missing if missing == Classes::default() => GateCall::Allow,
            missing => GateCall::Prompt(missing),
        },
    }
}

pub fn blocked_notice(
    thread: Option<&str>,
    worker: &str,
    content: Option<&BlockedContent>,
) -> String {
    let label = thread.map_or_else(|| worker.to_string(), |name| format!("{name}/{worker}"));
    match content {
        Some(content) => {
            let options: Vec<&str> = content
                .options
                .iter()
                .map(|option| option.option_id.as_str())
                .collect();
            format!(
                "{label} blocked: {} {} request {} options {}",
                content.tool,
                content.summary,
                content.request_id,
                options.join("|")
            )
        }
        None => format!("{label} permission settled"),
    }
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

    fn ask(tool: &str, input: serde_json::Value) -> ClaudePermission {
        ClaudePermission {
            input,
            content: BlockedContent {
                request_id: "r".into(),
                tool: tool.into(),
                summary: String::new(),
                options: Vec::new(),
                pending: Vec::new(),
            },
        }
    }

    #[test]
    fn mcptools_decisions_fail_closed() {
        use McptoolsDecision::*;
        for (tool, input, want) in [
            ("mcp__mcptools__execute", json!({"code": "read"}), Read),
            (
                "mcp__mcptools__call_tool",
                json!({"allowWrites": false, "allowSpend": false}),
                Read,
            ),
            (
                "mcp__mcptools__execute",
                json!({"allowWrites": true}),
                Elevated(Classes {
                    writes: true,
                    spend: false,
                }),
            ),
            (
                "mcp__mcptools__call_tool",
                json!({"allowSpend": true}),
                Elevated(Classes {
                    writes: false,
                    spend: true,
                }),
            ),
            ("mcp__mcptools__execute", json!({"allowSpend": null}), Deny),
            (
                "mcp__mcptools__call_tool",
                json!({"allowWrites": "false"}),
                Deny,
            ),
            ("mcp__mcptools__find_tools", json!({}), Other),
            ("Bash", json!({}), Other),
        ] {
            assert_eq!(
                mcptools_decision(&ask(tool, input.clone())),
                want,
                "{tool} {input}"
            );
        }
    }

    fn gate(grants: Grants, tool: &str, input: serde_json::Value) -> GateCall {
        mcptools_gate(grants, &ask(tool, input))
    }

    const WRITES: Grants = Grants {
        writes: true,
        spend: false,
    };
    const SPEND: Grants = Grants {
        writes: false,
        spend: true,
    };
    const BOTH: Grants = Grants {
        writes: true,
        spend: true,
    };

    #[test]
    fn gate_prompts_for_each_ungranted_class() {
        let none = Grants::default();
        let execute = "mcp__mcptools__execute";
        assert_eq!(
            gate(none, execute, json!({"allowWrites": true})),
            GateCall::Prompt(Classes {
                writes: true,
                spend: false
            })
        );
        assert_eq!(
            gate(none, execute, json!({"allowSpend": true})),
            GateCall::Prompt(Classes {
                writes: false,
                spend: true
            })
        );
        assert_eq!(
            gate(
                none,
                execute,
                json!({"allowWrites": true, "allowSpend": true})
            ),
            GateCall::Prompt(Classes {
                writes: true,
                spend: true
            })
        );
    }

    #[test]
    fn gate_allows_only_when_every_needed_class_is_granted() {
        let call = "mcp__mcptools__call_tool";
        let both = json!({"allowWrites": true, "allowSpend": true});
        assert_eq!(
            gate(WRITES, call, json!({"allowWrites": true})),
            GateCall::Allow
        );
        assert_eq!(
            gate(SPEND, call, json!({"allowSpend": true})),
            GateCall::Allow
        );
        assert_eq!(gate(BOTH, call, both.clone()), GateCall::Allow);
        assert_eq!(
            gate(WRITES, call, both),
            GateCall::Prompt(Classes {
                writes: false,
                spend: true
            })
        );
        assert_eq!(
            gate(SPEND, call, json!({"allowWrites": true})),
            GateCall::Prompt(Classes {
                writes: true,
                spend: false
            })
        );
    }

    #[test]
    fn gate_allows_reads_denies_malformed_and_skips_other_tools() {
        let execute = "mcp__mcptools__execute";
        for grants in [Grants::default(), BOTH] {
            assert_eq!(
                gate(grants, execute, json!({"code": "read"})),
                GateCall::Allow
            );
            assert_eq!(
                gate(grants, execute, json!({"allowWrites": "true"})),
                GateCall::Deny
            );
            assert_eq!(
                gate(grants, execute, json!({"allowSpend": null})),
                GateCall::Deny
            );
            assert_eq!(gate(grants, "Bash", json!({})), GateCall::Other);
        }
    }

    #[test]
    fn yolo_grants_both_classes_and_ask_grants_none() {
        assert_eq!(Grants::for_mode(PermissionMode::Yolo), BOTH);
        assert_eq!(Grants::for_mode(PermissionMode::Ask), Grants::default());
        assert_eq!(WRITES.union(SPEND), BOTH);
    }

    #[test]
    fn grants_parse_missing_valid_and_invalid() {
        assert_eq!(parse_grants(None), (Grants::default(), None));
        assert_eq!(
            parse_grants(Some("{\"writes\":true,\"spend\":false}")),
            (WRITES, None)
        );
        assert_eq!(parse_grants(Some("{\"spend\":true}")), (SPEND, None));
        let (grants, warning) = parse_grants(Some("not json"));
        assert_eq!(grants, Grants::default());
        assert!(warning.is_some());
    }

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
