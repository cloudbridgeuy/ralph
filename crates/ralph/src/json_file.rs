use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

pub(crate) fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("could not encode {}: {error}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut temporary = NamedTempFile::with_prefix_in(".ralph-tmp", dir).map_err(|error| {
        format!(
            "could not create a temporary file in {}: {error}",
            dir.display()
        )
    })?;
    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .ok();
    }
    writeln!(temporary, "{text}")
        .map_err(|error| format!("could not write {}: {error}", temporary.path().display()))?;
    temporary
        .persist(path)
        .map(drop)
        .map_err(|error| format!("could not replace {}: {}", path.display(), error.error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_json_does_not_share_one_temporary_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("records.json");
        fs::create_dir(path.with_extension("json.ralph-tmp")).unwrap();
        write_json(&path, &json!({"agents": []})).unwrap();
        assert_eq!(read_json(&path).unwrap(), json!({"agents": []}));
    }
}
