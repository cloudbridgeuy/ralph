use crate::private_file::check_path;
use ralph_core::chat_memory::{encode_usage, LedgerRow};
use ralph_core::worker::{decode_spawn_record, spawn_record_path};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub fn record_all(path: &Path, rows: impl IntoIterator<Item = LedgerRow>) -> io::Result<()> {
    rows.into_iter().try_for_each(|row| record(path, &row))
}

pub fn worker_ticket(store: Option<&Path>, worker: &str) -> Option<String> {
    let raw = fs::read_to_string(spawn_record_path(store?, worker)).ok()?;
    decode_spawn_record(&raw)?.ticket
}

pub fn record(path: &Path, row: &LedgerRow) -> io::Result<()> {
    let line = encode_usage(row).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    check_path(path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())
}

#[cfg(test)]
pub(crate) mod fixtures {
    use ralph_core::worker::spawn_record_path;
    use std::fs;
    use std::path::Path;

    pub(crate) fn spawn_record(store: &Path, worker: &str, ticket: &str) {
        let path = spawn_record_path(store, worker);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!(r#"{{"ticket":"{ticket}","tracker":"linear","at":0}}"#),
        )
        .unwrap();
    }
}
