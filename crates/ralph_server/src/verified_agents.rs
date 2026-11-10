use crate::json_file::{read_json, write_json};
use crate::paths::config_dir;
use ralph_core::worker::{
    drift_lock_file, merge_verified, record_outcome, stamp_check, DriftEntry, DriftRecord,
    VerifiedAgent, VerifiedAgents, DRIFT_FILE, VERIFIED_AGENTS_FILE,
};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

pub(crate) fn load() -> VerifiedAgents {
    read_json(&config_dir().join(VERIFIED_AGENTS_FILE))
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(crate) fn save(entry: VerifiedAgent) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let path = dir.join(VERIFIED_AGENTS_FILE);
    let existing = match fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let value = serde_json::to_value(merge_verified(existing.as_deref(), entry)?)
        .map_err(|error| format!("could not encode {VERIFIED_AGENTS_FILE}: {error}"))?;
    write_json(&path, &value)
}

pub(crate) fn load_drift() -> DriftRecord {
    read_json(&config_dir().join(DRIFT_FILE))
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(crate) fn save_drift(entry: DriftEntry) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let value = serde_json::to_value(record_outcome(&load_drift(), entry))
        .map_err(|error| format!("could not encode {DRIFT_FILE}: {error}"))?;
    write_json(&dir.join(DRIFT_FILE), &value)
}

pub(crate) fn save_last_check(now: chrono::DateTime<chrono::Utc>) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let value = serde_json::to_value(stamp_check(&load_drift(), now))
        .map_err(|error| format!("could not encode {DRIFT_FILE}: {error}"))?;
    write_json(&dir.join(DRIFT_FILE), &value)
}

pub(crate) fn drift_lock_path(agent: &str) -> PathBuf {
    config_dir().join(drift_lock_file(agent))
}
