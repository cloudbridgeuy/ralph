use super::*;
use crate::permission_queue::PermissionQueue;
use ralph_core::worker::{unattended_acp, PermissionOption};

#[derive(Debug, Clone, serde::Serialize, agent_client_protocol::JsonRpcRequest)]
#[serde(transparent)]
#[request(method = "session/request_permission", response = RequestPermissionResponse)]
pub(super) struct RawPermissionRequest(pub(super) serde_json::Value);

impl RawPermissionRequest {
    fn content(&self, agy: bool) -> BlockedContent {
        if agy {
            return permission_content(&self.0);
        }
        let normalized = serde_json::from_value::<RequestPermissionRequest>(self.0.clone())
            .ok()
            .and_then(|request| serde_json::to_value(request).ok())
            .unwrap_or_default();
        permission_content(&normalized)
    }
}

impl<'de> serde::Deserialize<'de> for RawPermissionRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
        serde_json::from_value::<RequestPermissionRequest>(value.clone())
            .map_err(serde::de::Error::custom)?;
        Ok(Self(value))
    }
}

type Reply = Responder<RequestPermissionResponse>;

fn selected_response(command: WorkerCommand) -> Option<RequestPermissionResponse> {
    match encode(command)? {
        AcpCall::PermissionResponse(option_id) => Some(RequestPermissionResponse::new(
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id)),
        )),
        _ => None,
    }
}

fn respond_cancelled(responder: Reply) {
    responder
        .respond(RequestPermissionResponse::new(
            RequestPermissionOutcome::Cancelled,
        ))
        .ok();
}

fn respond_selected(responder: Reply, option_id: String) {
    let request_id = responder.id().to_string();
    let answer = PermissionAnswer {
        request_id,
        option_id,
    };
    if let Some(response) = selected_response(WorkerCommand::AnswerPermission(answer)) {
        responder.respond(response).ok();
    }
}

fn auto_option<'a>(
    agy: bool,
    mode: PermissionMode,
    value: &serde_json::Value,
    content: &'a BlockedContent,
) -> Option<&'a PermissionOption> {
    if agy {
        agy_auto_allow_option(value, content)
    } else {
        auto_allow_option(mode, content)
    }
}

pub(super) struct Permissions {
    mode: PermissionMode,
    agy: bool,
    scheduled: bool,
    queue: PermissionQueue<Reply>,
    pub(super) heartbeat: Arc<Heartbeat>,
}

impl Permissions {
    pub(super) fn new(
        markers: &WorkerMarkers,
        mode: PermissionMode,
        agy: bool,
        scheduled: bool,
        queue: PermissionQueue<Reply>,
    ) -> Arc<Self> {
        Arc::new(Self {
            mode,
            agy,
            scheduled,
            queue,
            heartbeat: Heartbeat::new(markers.activity.clone(), markers.gone.clone()),
        })
    }

    pub(super) fn push(&self, request: &RawPermissionRequest, responder: Reply) {
        let value = &request.0;
        let content = BlockedContent {
            request_id: responder.id().to_string(),
            ..request.content(self.agy)
        };
        if self.scheduled {
            return self.refuse_unattended(value, &content, responder);
        }
        let option = auto_option(self.agy, self.mode, value, &content);
        if let Some(option) = option {
            respond_selected(responder, option.option_id.clone());
            return;
        }
        self.queue.enqueue(responder, content);
    }

    fn refuse_unattended(
        &self,
        request: &serde_json::Value,
        content: &BlockedContent,
        responder: Reply,
    ) {
        let verdict = unattended_acp(self.agy, request, content);
        self.queue.report_unattended(&verdict, content);
        match verdict.option_id() {
            Some(option_id) => respond_selected(responder, option_id.to_string()),
            None => respond_cancelled(responder),
        }
    }

    pub(super) fn answer(&self, answer: &PermissionAnswer) -> Result<(), String> {
        let (responder, option) = self
            .queue
            .take_answer(&answer.request_id, &answer.option_id)?;
        respond_selected(responder, option.option_id);
        self.heartbeat.touch();
        Ok(())
    }

    pub(super) fn cancel_all(&self) {
        for responder in self.queue.drain() {
            respond_cancelled(responder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::JsonRpcMessage;
    use serde_json::json;

    #[test]
    fn cursor_keeps_sdk_normalized_permission() -> Result<(), Box<dyn std::error::Error>> {
        let params = json!({
            "sessionId": "session",
            "toolCall": {"toolCallId": "call", "name": "ralph-worker-report: report", "kind": "execute", "title": "Run command"},
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"}]
        });
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        let content = request.content(false);
        let typed: RequestPermissionRequest = serde_json::from_value(params)?;
        let original = permission_content(&serde_json::to_value(typed)?);
        assert_eq!(content.render(), original.render());
        for mode in [PermissionMode::Ask, PermissionMode::Yolo] {
            assert_eq!(
                auto_option(false, mode, &request.0, &content),
                auto_allow_option(mode, &original)
            );
        }
        Ok(())
    }

    #[test]
    fn decoded_agy_permission_requires_exact_tool_identity(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut params = json!({
            "sessionId": "session",
            "toolCall": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_1",
                "name": "mcp_ralph-worker_report",
                "args": {"text": "hello"},
                "status": "pending",
                "kind": "other",
                "title": "report",
                "rawInput": {"text": "hello"}
            },
            "options": [
                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                {"optionId": "deny", "name": "Deny", "kind": "reject_once"}
            ]
        });
        for name in ["mcp_ralph-worker_report", "mcp_ralph-worker_set_status"] {
            params["toolCall"]["name"] = json!(name);
            let request =
                RawPermissionRequest::parse_message("session/request_permission", &params)?;
            let content = permission_content(&request.0);
            for mode in [PermissionMode::Ask, PermissionMode::Yolo] {
                assert_eq!(
                    auto_option(true, mode, &request.0, &content).map(|o| o.option_id.as_str()),
                    Some("allow")
                );
            }
        }
        for name in [
            "mcp_evil_report",
            "mcp_ralph-worker_report_extra",
            "report",
            "write_file",
            "ask_permission",
        ] {
            params["toolCall"]["name"] = json!(name);
            params["toolCall"]["title"] = json!("ralph-worker-report: report");
            params["toolCall"]["args"] = json!({"name": "mcp_ralph-worker_report", "action": "mcp", "target": "ralph-worker/report"});
            let request =
                RawPermissionRequest::parse_message("session/request_permission", &params)?;
            for mode in [PermissionMode::Ask, PermissionMode::Yolo] {
                assert!(
                    auto_option(true, mode, &request.0, &permission_content(&request.0)).is_none()
                );
            }
        }
        params["toolCall"]
            .as_object_mut()
            .ok_or("toolCall is not an object")?
            .remove("name");
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        assert!(agy_auto_allow_option(&request.0, &permission_content(&request.0)).is_none());
        params["toolCall"]["server"] = json!("ralph-worker");
        params["toolCall"]["tool"] = json!("report");
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        assert!(agy_auto_allow_option(&request.0, &permission_content(&request.0)).is_some());
        let mut no_allow = permission_content(&request.0);
        no_allow
            .options
            .retain(|option| option.kind != "allow_once");
        assert!(agy_auto_allow_option(&request.0, &no_allow).is_none());
        params["toolCall"]["name"] = json!("report");
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        assert!(agy_auto_allow_option(&request.0, &permission_content(&request.0)).is_some());
        params["toolCall"]
            .as_object_mut()
            .ok_or("toolCall is not an object")?
            .remove("tool");
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        assert!(agy_auto_allow_option(&request.0, &permission_content(&request.0)).is_some());
        params["toolCall"]["name"] = json!("mcp_ralph-worker_report");
        params["toolCall"]["server"] = json!("evil");
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        assert!(agy_auto_allow_option(&request.0, &permission_content(&request.0)).is_none());
        params["toolCall"]["server"] = json!(null);
        let request = RawPermissionRequest::parse_message("session/request_permission", &params)?;
        assert!(agy_auto_allow_option(&request.0, &permission_content(&request.0)).is_none());
        assert!(RawPermissionRequest::parse_message(
            "session/request_permission",
            &json!({"toolCall": {}})
        )
        .is_err());
        Ok(())
    }
}
