use super::Opening;
use crate::watches::locked;
use agent_client_protocol::schema::v1::{
    CloseSessionRequest, LoadSessionRequest, NewSessionRequest, SessionId, SessionModeState,
    SetSessionConfigOptionRequest, SetSessionModeRequest,
};
use agent_client_protocol::{Agent, ConnectionTo, UntypedMessage};
use futures::{channel::oneshot, FutureExt};
use ralph_core::worker::{
    agy_effort_available, agy_effort_confirmed, agy_model_base, agy_selection, PermissionMode,
    AUTH_REQUIRED_CODE,
};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Selection {
    pub(super) model: String,
    pub(super) effort: Option<String>,
}

type Pending = Option<(String, String, oneshot::Sender<serde_json::Value>)>;
pub(super) type Updates = Arc<Mutex<Pending>>;

pub(super) fn observe(updates: &Updates, session: &str, update: &serde_json::Value) {
    let mut pending = locked(updates);
    let Some((wanted, model, _)) = pending.as_ref() else {
        return;
    };
    let matches = wanted == session
        && update["sessionUpdate"] == "config_option_update"
        && update["configOptions"].as_array().is_some_and(|options| {
            options
                .iter()
                .any(|option| option["id"] == "model" && option["currentValue"] == *model)
        });
    if matches {
        if let Some((_, _, sender)) = pending.take() {
            sender.send(update.clone()).ok();
        }
    }
}

pub(super) fn command(program: &str, mode: PermissionMode) -> Command {
    let mut command = Command::new(program);
    command.arg("acp");
    if program == "agy" && mode == PermissionMode::Yolo {
        command.arg("--dangerously-skip-permissions");
    }
    command
}

pub(super) fn restore_default(mode: PermissionMode, current: Option<&str>) -> Result<bool, String> {
    if mode == PermissionMode::Yolo {
        return Ok(false);
    }
    current
        .map(|current| current != "default")
        .ok_or_else(|| "agy did not report its session mode".to_string())
}

pub(super) async fn apply_mode(
    connection: &ConnectionTo<Agent>,
    session: &SessionId,
    mode: PermissionMode,
    state: Option<&SessionModeState>,
) -> Result<(), String> {
    let current = state.map(|state| state.current_mode_id.0.as_ref());
    if restore_default(mode, current)? {
        if !state.is_some_and(|state| {
            state
                .available_modes
                .iter()
                .any(|item| item.id.0.as_ref() == "default")
        }) {
            return Err("agy did not advertise its default session mode".to_string());
        }
        connection
            .send_request(SetSessionModeRequest::new(session.clone(), "default"))
            .block_task()
            .await
            .map_err(|error| {
                format!(
                    "agy could not restore default session mode: {}",
                    error.message
                )
            })?;
    }
    Ok(())
}

pub(super) fn session_failure(code: i32, message: &str) -> String {
    if code == AUTH_REQUIRED_CODE {
        "agy is not authenticated; sign in with `agy` and retry".into()
    } else {
        format!("agy session request failed: {message}")
    }
}

pub(super) async fn open(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
) -> Result<(SessionId, Selection), String> {
    let request =
        NewSessionRequest::new(opening.cwd.clone()).mcp_servers(vec![opening.server.clone()]);
    let response = connection
        .send_request(UntypedMessage::new("session/new", request).map_err(|e| e.to_string())?)
        .block_task()
        .await
        .map_err(|error| session_failure(i32::from(error.code), &error.message))?;
    let id = SessionId::new(
        response["sessionId"]
            .as_str()
            .ok_or("agy session response has no id")?
            .to_string(),
    );
    let result = apply(connection, &id, opening, &response).await;
    if result.is_err() {
        connection
            .send_request(CloseSessionRequest::new(id.clone()))
            .block_task()
            .await
            .ok();
    }
    result.map(|selection| (id, selection))
}

pub(super) async fn load(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    id: SessionId,
) -> Result<(SessionId, Selection), String> {
    let request = LoadSessionRequest::new(id.clone(), opening.cwd.clone())
        .mcp_servers(vec![opening.server.clone()]);
    let response = connection
        .send_request(UntypedMessage::new("session/load", request).map_err(|e| e.to_string())?)
        .block_task()
        .await
        .map_err(|error| session_failure(i32::from(error.code), &error.message))?;
    let result = apply(connection, &id, opening, &response).await;
    if result.is_err() {
        connection
            .send_request(CloseSessionRequest::new(id.clone()))
            .block_task()
            .await
            .ok();
    }
    result.map(|selection| (id, selection))
}

async fn apply(
    connection: &ConnectionTo<Agent>,
    id: &SessionId,
    opening: &Opening,
    initial: &serde_json::Value,
) -> Result<Selection, String> {
    let mode = initial
        .get("modes")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    apply_mode(connection, id, opening.permission_mode, mode.as_ref()).await?;
    let model = agy_selection(opening.model.as_deref(), opening.effort.as_deref(), initial)?;
    let mut state = initial.clone();
    if let Some(ref model) = model {
        let primary = agy_model_base(model, initial);
        let same = initial["models"]["currentModelId"] == *model;
        let (send, receive) = oneshot::channel();
        *locked(&opening.updates) = Some((id.to_string(), primary.to_string(), send));
        let request = UntypedMessage::new(
            "session/set_model",
            serde_json::json!({"sessionId": id, "modelId": model}),
        )
        .map_err(|e| e.to_string())?;
        let set = connection
            .send_request(request)
            .block_task()
            .await
            .map_err(|e| format!("agy could not set model: {}", e.message));
        if set.is_err() {
            locked(&opening.updates).take();
        }
        set?;
        if !same {
            let timeout = blocking::unblock(|| std::thread::sleep(Duration::from_secs(8))).fuse();
            futures::pin_mut!(timeout);
            state = futures::select! { update = receive.fuse() => update.map_err(|_| "agy model update was lost".to_string())?, () = timeout => return Err("agy did not send updated model settings".to_string()), };
        }
        locked(&opening.updates).take();
        if let Some(suffix) = model
            .strip_prefix(primary)
            .and_then(|tail| tail.strip_prefix('-'))
        {
            if ["low", "medium", "high", "xhigh"].contains(&suffix)
                && !state["configOptions"].as_array().is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item["id"] == "thought_level" && item["currentValue"] == suffix)
                })
            {
                return Err(format!("agy did not confirm model '{model}'"));
            }
        }
    }
    agy_effort_available(opening.effort.as_deref(), &state)?;
    if let Some(effort) = opening.effort.as_deref() {
        let model = state["configOptions"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == "model"))
            .and_then(|item| item["currentValue"].as_str())
            .ok_or("agy did not report its current model")?;
        let result = connection
            .send_request(SetSessionConfigOptionRequest::new(
                id.clone(),
                "thought_level",
                effort,
            ))
            .block_task()
            .await
            .map_err(|error| format!("agy could not set effort: {}", error.message))?;
        let options = serde_json::to_value(result).map_err(|error| error.to_string())?;
        if !agy_effort_confirmed(&options, model, effort) {
            return Err("agy did not confirm the requested model and effort".to_string());
        }
        state = options;
    }
    selection(initial, &state, model.as_deref())
}

pub(super) fn selection(
    initial: &serde_json::Value,
    state: &serde_json::Value,
    chosen: Option<&str>,
) -> Result<Selection, String> {
    let current = |id: &str| {
        state["configOptions"]
            .as_array()?
            .iter()
            .find(|item| item["id"] == id)?["currentValue"]
            .as_str()
            .map(str::to_string)
    };
    let effort = current("thought_level");
    let model = chosen
        .or_else(|| initial["models"]["currentModelId"].as_str())
        .ok_or("agy did not report its current model")?;
    let base = current("model");
    let candidate = base
        .as_ref()
        .zip(effort.as_ref())
        .map(|(base, effort)| format!("{base}-{effort}"));
    let model = candidate
        .filter(|candidate| {
            initial["models"]["availableModels"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["modelId"] == *candidate))
        })
        .unwrap_or_else(|| model.to_string());
    Ok(Selection { model, effort })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agy_yolo_uses_only_the_permission_flag() {
        let ask = command("agy", PermissionMode::Ask);
        let yolo = command("agy", PermissionMode::Yolo);
        let cursor = command("agent", PermissionMode::Yolo);
        assert_eq!(ask.get_args().collect::<Vec<_>>(), ["acp"]);
        assert_eq!(cursor.get_args().collect::<Vec<_>>(), ["acp"]);
        assert_eq!(
            yolo.get_args().collect::<Vec<_>>(),
            ["acp", "--dangerously-skip-permissions"]
        );
    }
    #[test]
    fn ask_restores_default_and_rejects_missing_mode() {
        assert_eq!(
            restore_default(PermissionMode::Ask, Some("default")),
            Ok(false)
        );
        assert_eq!(restore_default(PermissionMode::Ask, Some("auto")), Ok(true));
        assert!(restore_default(PermissionMode::Ask, None).is_err());
        assert_eq!(
            restore_default(PermissionMode::Yolo, Some("auto")),
            Ok(false)
        );
    }

    #[test]
    fn model_updates_are_matched_by_session_and_model() {
        let updates = Arc::new(Mutex::new(None));
        let (send, mut receive) = oneshot::channel();
        *locked(&updates) = Some(("s1".into(), "flash".into(), send));
        let update = serde_json::json!({"sessionUpdate":"config_option_update","configOptions":[{"id":"model","currentValue":"flash"}]});
        observe(&updates, "other", &update);
        observe(
            &updates,
            "s1",
            &serde_json::json!({"sessionUpdate":"config_option_update","configOptions":[{"id":"model","currentValue":"other"}]}),
        );
        assert!(matches!(receive.try_recv(), Ok(None)));
        observe(&updates, "s1", &update);
        assert!(matches!(receive.try_recv(), Ok(Some(value)) if value == update));
    }
}
