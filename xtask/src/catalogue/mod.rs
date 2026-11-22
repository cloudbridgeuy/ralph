use std::fs;
use std::path::{Path, PathBuf};

mod core;

#[derive(Debug, clap::Parser)]
pub struct CatalogueCommand {
    #[arg(long)]
    pub store: PathBuf,

    #[arg(long, default_value = "crates/ralph_api/fixtures/catalogue.json")]
    pub out: PathBuf,
}

fn read_or_empty(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("could not read {}: {error}", path.display()).into()),
    }
}

fn read_statuses(dir: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths.iter().map(|path| read_or_empty(path)).collect()
}

fn out_is_inside_store(out: &Path, store: &Path) -> bool {
    out.starts_with(store)
}

fn canonical_target(path: &Path) -> std::io::Result<PathBuf> {
    let mut tail = Vec::new();
    let mut base = path;
    loop {
        match base.canonicalize() {
            Ok(resolved) => {
                return Ok(tail.iter().rev().fold(resolved, |acc, part| acc.join(part)))
            }
            Err(error) => {
                let (Some(parent), Some(name)) = (base.parent(), base.file_name()) else {
                    return Err(error);
                };
                tail.push(name.to_owned());
                base = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
            }
        }
    }
}

pub fn run(command: CatalogueCommand) -> Result<(), Box<dyn std::error::Error>> {
    let store = command
        .store
        .canonicalize()
        .map_err(|error| format!("could not resolve {}: {error}", command.store.display()))?;
    let out = canonical_target(&command.out)
        .map_err(|error| format!("could not resolve {}: {error}", command.out.display()))?;
    if out_is_inside_store(&out, &store) {
        return Err(format!(
            "--out {} is inside --store {}; the store is never written",
            command.out.display(),
            command.store.display()
        )
        .into());
    }
    let log = read_or_empty(&command.store.join("log.jsonl"))?;
    let usage = read_or_empty(&command.store.join("usage.jsonl"))?;
    let statuses = read_statuses(&command.store.join("status"))?;
    let status_refs: Vec<&str> = statuses.iter().map(String::as_str).collect();
    let text = core::render(&core::derive(&log, &usage, &status_refs))?;
    if let Some(parent) = command.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&command.out, &text)?;
    println!("wrote {} ({} bytes)", command.out.display(), text.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::out_is_inside_store;
    use std::path::Path;

    #[test]
    fn an_out_path_under_the_store_is_refused() {
        let store = Path::new("/data/store");
        assert!(out_is_inside_store(Path::new("/data/store/a.json"), store));
        assert!(out_is_inside_store(
            Path::new("/data/store/x/y.json"),
            store
        ));
        assert!(out_is_inside_store(Path::new("/data/store"), store));
    }

    #[test]
    fn an_out_path_beside_the_store_is_allowed() {
        let store = Path::new("/data/store");
        assert!(!out_is_inside_store(
            Path::new("/data/store2/a.json"),
            store
        ));
        assert!(!out_is_inside_store(Path::new("/data/a.json"), store));
        assert!(!out_is_inside_store(Path::new("/tmp/a.json"), store));
    }
}
