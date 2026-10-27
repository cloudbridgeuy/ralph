use crate::json_file::{read_json, write_json};
use crate::paths::config_dir;
use ralph_core::worker::{VerifiedAgent, VerifiedAgents, VERIFIED_AGENTS_FILE};
use std::fs;

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
    let value = serde_json::to_value(load().record(entry))
        .map_err(|error| format!("could not encode {VERIFIED_AGENTS_FILE}: {error}"))?;
    write_json(&dir.join(VERIFIED_AGENTS_FILE), &value)
}
