use ralph_core::wire::live::live_row;
use ralph_core::wire::record::{CallRecord, SessionRecord};
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
