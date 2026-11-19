use agent_client_protocol::schema::v1::{SessionId, SessionModeState, SetSessionModeRequest};
use agent_client_protocol::{Agent, ConnectionTo};
use ralph_core::worker::PermissionMode;
use std::process::Command;

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
}
