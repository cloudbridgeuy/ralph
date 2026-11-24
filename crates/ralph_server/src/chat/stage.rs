use crate::chat::memory::Store;
use crate::json_file::{read_json, write_json};
use ralph_core::chat_local::notice_record;
use ralph_core::chat_memory::Role;
use ralph_core::chat_socket::SocketRequest;
use ralph_core::chat_stage::{
    add, close, expire, parse_add, parse_done, render, Actor, AuditRow, Op, Stage,
};
use ralph_core::chat_stage::{STAGE_ADD, STAGE_DONE};
use ralph_core::chat_status::Status;
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const STAGE_FILE: &str = "stage.json";
const AUDIT_FILE: &str = "stage-audit.jsonl";

pub fn load(dir: &Path) -> Result<Stage, String> {
    let path = dir.join(STAGE_FILE);
    if !path.exists() {
        return Ok(Stage::default());
    }
    serde_json::from_value(read_json(&path)?)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

pub fn save(dir: &Path, stage: &Stage) -> Result<(), String> {
    let value = serde_json::to_value(stage).map_err(|error| error.to_string())?;
    write_json(&dir.join(STAGE_FILE), &value)
}

pub fn append_audit(dir: &Path, rows: &[AuditRow]) -> Result<(), String> {
    let mut text = String::new();
    for row in rows {
        text.push_str(&serde_json::to_string(row).map_err(|error| error.to_string())?);
        text.push('\n');
    }
    let path = dir.join(AUDIT_FILE);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

pub fn rendered(dir: &Path, status: &Status) -> Result<String, String> {
    Ok(render(&load(dir)?.items, status))
}

pub fn call(store: &Store, request: &SocketRequest) -> Option<Result<String, String>> {
    match request.tool.as_str() {
        STAGE_ADD => Some(tool_add(store, &request.args)),
        STAGE_DONE => Some(tool_done(store, &request.args)),
        _ => None,
    }
}

fn tool_add(store: &Store, args: &Value) -> Result<String, String> {
    let new = parse_add(args)?;
    let dir = store.dir();
    let log_len = store.log_len().map_err(|error| error.to_string())?;
    let ts = chrono::Utc::now().to_rfc3339();
    let (stage, item) = add(load(dir)?, new, &ts, log_len);
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    save(dir, &stage)?;
    append_audit(dir, &[AuditRow::new(&ts, Actor::Model, Op::Add, &item)])?;
    Ok(format!("{} added", item.id))
}

fn tool_done(store: &Store, args: &Value) -> Result<String, String> {
    let closing = parse_done(args)?;
    let dir = store.dir();
    let (stage, item) = close(load(dir)?, &closing.id)?;
    save(dir, &stage)?;
    let ts = chrono::Utc::now().to_rfc3339();
    let row = AuditRow::new(&ts, Actor::Model, Op::Done, &item).with_note(closing.note);
    append_audit(dir, &[row])?;
    Ok(format!("{} done", item.id))
}

pub fn expire_due(store: &Store, dir: &Path) -> Result<(), String> {
    let log_len = store.log_len().map_err(|error| error.to_string())?;
    let (kept, gone) = expire(load(dir)?, log_len);
    if gone.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    save(dir, &kept)?;
    let ts = chrono::Utc::now().to_rfc3339();
    let rows: Vec<AuditRow> = gone
        .iter()
        .map(|item| AuditRow::new(&ts, Actor::Expiry, Op::Expire, item))
        .collect();
    append_audit(dir, &rows)?;
    gone.iter().try_for_each(|item| {
        store
            .append(
                Role::User,
                &notice_record(&format!("Stage: {} expired: {}", item.id, item.text)),
            )
            .map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests;
