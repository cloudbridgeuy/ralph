use crate::private_file::check_path;
use ralph_core::chat_memory::{encode_usage, LedgerRow};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub fn record(path: &Path, row: &LedgerRow) -> io::Result<()> {
    let line = encode_usage(row).map_err(io::Error::other)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    check_path(path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())
}
