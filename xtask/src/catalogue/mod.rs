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

pub fn run(command: CatalogueCommand) -> Result<(), Box<dyn std::error::Error>> {
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
