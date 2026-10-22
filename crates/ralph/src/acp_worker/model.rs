use super::Opening;
use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigSelectOptions, SessionId,
    SetSessionConfigOptionRequest,
};
use agent_client_protocol::{Agent, ConnectionTo};
use ralph_core::worker::{
    cursor_model_rejection, restore_json_keys, select_cursor_model, AdvertisedCursorModel,
    CursorVariantRequest, CURSOR_ACP_DEFAULT_KEYS, CURSOR_CLI_DEFAULT_KEYS,
};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

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
    let choices: Vec<AdvertisedCursorModel> = owned
        .iter()
        .map(|(name, value)| AdvertisedCursorModel { name, value })
        .collect();
    let value = select_cursor_model(
        &CursorVariantRequest {
            model: requested,
            reasoning_effort: opening.reasoning_effort.as_deref(),
            fast: opening.fast,
        },
        &choices,
    )
    .map_err(|detail| cursor_model_rejection(opening.agent.name, requested, &detail, None))?;
    let snapshot = snapshot_cursor_default()?;
    let set = connection
        .send_request(SetSessionConfigOptionRequest::new(
            session.clone(),
            "model",
            value.as_str(),
        ))
        .block_task()
        .await;
    let restored = restore_cursor_default(&snapshot);
    match (set, restored) {
        (Err(error), Ok(())) => Err(cursor_model_rejection(
            opening.agent.name,
            requested,
            &error.message,
            error
                .data
                .as_ref()
                .and_then(|data| data.get("message"))
                .and_then(|message| message.as_str()),
        )),
        (Err(error), Err(restore)) => Err(format!(
            "{}. The user's Cursor model default could not be restored: {restore}",
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
        )),
        (Ok(_), Err(restore)) => Err(format!(
            "cursor model '{requested}' was set for this session, but the user's Cursor model default could not be restored: {restore}"
        )),
        (Ok(_), Ok(())) => Ok(()),
    }
}

struct CursorDefaultSnapshot {
    cli_path: PathBuf,
    cli: Value,
    acp_path: PathBuf,
    acp: Option<Value>,
}

fn snapshot_cursor_default() -> Result<CursorDefaultSnapshot, String> {
    let dir = cursor_dir()?;
    let cli_path = dir.join("cli-config.json");
    let acp_path = dir.join("acp-config.json");
    let cli = read_json(&cli_path)?;
    let acp = if acp_path.is_file() {
        Some(read_json(&acp_path)?)
    } else {
        None
    };
    Ok(CursorDefaultSnapshot {
        cli_path,
        cli,
        acp_path,
        acp,
    })
}

fn restore_cursor_default(snapshot: &CursorDefaultSnapshot) -> Result<(), String> {
    restore_file(&snapshot.cli_path, &snapshot.cli, CURSOR_CLI_DEFAULT_KEYS)?;
    match &snapshot.acp {
        Some(before) => restore_file(&snapshot.acp_path, before, CURSOR_ACP_DEFAULT_KEYS),
        None if snapshot.acp_path.is_file() => restore_file(
            &snapshot.acp_path,
            &serde_json::json!({}),
            CURSOR_ACP_DEFAULT_KEYS,
        ),
        None => Ok(()),
    }
}

fn restore_file(path: &Path, before: &Value, keys: &[&str]) -> Result<(), String> {
    let after = read_json(path)?;
    let restored = restore_json_keys(before, &after, keys)?;
    if restored == after {
        return Ok(());
    }
    write_json(path, &restored)
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("could not encode {}: {error}", path.display()))?;
    let temporary = path.with_extension("json.ralph-tmp");
    fs::write(&temporary, format!("{text}\n"))
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, path).map_err(|error| {
        fs::remove_file(&temporary).ok();
        format!("could not replace {}: {error}", path.display())
    })
}

fn cursor_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(PathBuf::from(home).join(".cursor"))
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
            .map(|item| (item.name.clone(), item.value.to_string()))
            .collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .map(|item| (item.name.clone(), item.value.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}
