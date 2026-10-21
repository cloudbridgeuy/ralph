use ralph_core::wire::live::live_row;
use ralph_core::wire::record::{CallRecord, SessionRecord};
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
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

pub fn read_session(root: &Path, id: &str) -> io::Result<(SessionRecord, Vec<CallRecord>)> {
    let dir = root.join(id);
    let meta_path = dir.join("session.json");
    let session = serde_json::from_slice(&std::fs::read(&meta_path)?)
        .map_err(|e| io::Error::other(format!("{}: {e}", meta_path.display())))?;
    let calls_path = dir.join("calls.jsonl");
    let mut calls = Vec::new();
    for (index, line) in BufReader::new(File::open(&calls_path)?).lines().enumerate() {
        let call = serde_json::from_str(&line?).map_err(|e| {
            io::Error::other(format!("{}:{}: {e}", calls_path.display(), index + 1))
        })?;
        calls.push(call);
    }
    Ok((session, calls))
}
