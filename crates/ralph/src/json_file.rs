use serde_json::Value;
use std::fs;
use std::path::Path;

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
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
