use crate::json_file::{read_json, write_json};
use crate::paths::config_dir;
use ralph_core::worker::{merge_verified, VerifiedAgent, VerifiedAgents, VERIFIED_AGENTS_FILE};
use std::fs;
use std::io::ErrorKind;

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
