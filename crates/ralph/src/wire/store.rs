use ralph_core::wire::live::live_row;
use ralph_core::wire::read::{parse_calls, parse_session};
use ralph_core::wire::record::{CallRecord, SessionRecord};
use std::error::Error;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::mpsc::Receiver;

pub fn open_session(dir: &Path, session: &SessionRecord) -> io::Result<File> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mut meta = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("session.json"))?;
    meta.write_all(&serde_json::to_vec_pretty(session).map_err(io::Error::other)?)?;
    meta.flush()?;
    OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(dir.join("calls.jsonl"))
}

fn append(file: &mut File, record: &CallRecord) -> io::Result<()> {
    let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}

pub fn write_loop(records: Receiver<CallRecord>, mut file: File) {
    for record in records {
        if let Err(error) = append(&mut file, &record) {
            eprintln!("ralph wire: failed to record call {}: {error}", record.seq);
        }
        if let Some(row) = live_row(&record) {
            println!("{row}");
        }
    }
}

pub fn list_sessions(root: &Path) -> io::Result<Vec<String>> {
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.path().join("session.json").is_file() {
            if let Some(id) = entry.file_name().to_str() {
                ids.push(id.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

pub fn read_session(
    root: &Path,
    id: &str,
) -> Result<(SessionRecord, Vec<CallRecord>), Box<dyn Error>> {
    let dir = root.join(id);
    let meta_path = dir.join("session.json");
    let session = parse_session(
        &meta_path.display().to_string(),
        &std::fs::read_to_string(&meta_path)?,
    )?;
    let calls_path = dir.join("calls.jsonl");
    let calls = parse_calls(
        &calls_path.display().to_string(),
        &std::fs::read_to_string(&calls_path)?,
    )?;
    Ok((session, calls))
}
