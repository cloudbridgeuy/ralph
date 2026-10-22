use super::Opening;
use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigSelectOptions, SessionId,
    SetSessionConfigOptionRequest,
};
use agent_client_protocol::{Agent, ConnectionTo};
use ralph_core::worker::{cursor_model_rejection, cursor_model_value, CursorModelChoice};

pub(super) async fn apply_model(
    connection: &ConnectionTo<Agent>,
    opening: &Opening,
    session: &SessionId,
    options: Option<&[SessionConfigOption]>,
) -> Result<(), String> {
    let Some(requested) = opening.model.as_deref() else {
        return Ok(());
    };
    let owned = model_choices(options.unwrap_or(&[]));
    let choices: Vec<CursorModelChoice> = owned
        .iter()
        .map(|(value, name)| CursorModelChoice { value, name })
        .collect();
    let value = cursor_model_value(requested, &choices);
    connection
        .send_request(SetSessionConfigOptionRequest::new(
            session.clone(),
            "model",
            value,
        ))
        .block_task()
        .await
        .map(|_| ())
        .map_err(|error| {
            cursor_model_rejection(
                opening.agent.name,
                requested,
                &error.message,
                error
                    .data
                    .as_ref()
                    .and_then(|data| data.get("message"))
                    .and_then(|message| message.as_str()),
            )
        })
}

fn model_choices(options: &[SessionConfigOption]) -> Vec<(String, String)> {
    let Some(option) = options
        .iter()
        .find(|option| option.id.to_string() == "model")
    else {
        return Vec::new();
    };
    let SessionConfigKind::Select(select) = &option.kind else {
        return Vec::new();
    };
    match &select.options {
        SessionConfigSelectOptions::Ungrouped(items) => items
            .iter()
            .map(|item| (item.value.to_string(), item.name.clone()))
            .collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .map(|item| (item.value.to_string(), item.name.clone()))
            .collect(),
        _ => Vec::new(),
    }
}
