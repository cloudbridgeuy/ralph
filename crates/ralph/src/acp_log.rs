use crate::cursor_owner::now_secs;
use ralph_core::worker::{acp_log_path, log_entry, rotated_log_path, rotates};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ChildStderr;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

pub struct CappedLog {
    path: Option<PathBuf>,
    open: Mutex<Option<(File, u64)>>,
}

impl CappedLog {
    pub fn new(store: Option<&Path>, id: &str) -> Arc<Self> {
        Arc::new(Self {
            path: store.map(|store| acp_log_path(store, id)),
            open: Mutex::new(None),
        })
    }

    pub fn append(&self, tag: &str, chunk: &[u8]) {
        let Some(path) = &self.path else { return };
        let entry = log_entry(now_secs(), tag, chunk);
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if open.is_none() {
            *open = open_log(path);
        }
        let Some((file, len)) = open.as_mut() else {
            return;
        };
        if rotates(*len, entry.len() as u64) {
            fs::rename(path, rotated_log_path(path)).ok();
            match open_log(path) {
                Some(fresh) => (*file, *len) = fresh,
                None => {
                    *open = None;
                    return;
                }
            }
        }
        if file.write_all(&entry).is_ok() {
            *len += entry.len() as u64;
        }
    }
}

fn open_log(path: &Path) -> Option<(File, u64)> {
    fs::create_dir_all(path.parent()?).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    let len = file.metadata().ok()?.len();
    Some((file, len))
}

pub struct LoggedRead<R> {
    pub inner: R,
    pub log: Arc<CappedLog>,
    pub tag: &'static str,
}

impl<R: Read> Read for LoggedRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0 {
            self.log.append(self.tag, &buf[..read]);
        }
        Ok(read)
    }
}

pub struct LoggedWrite<W> {
    pub inner: W,
    pub log: Arc<CappedLog>,
    pub tag: &'static str,
}

impl<W: Write> Write for LoggedWrite<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        if written > 0 {
            self.log.append(self.tag, &buf[..written]);
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub fn drain_stderr(stderr: ChildStderr, log: Arc<CappedLog>) {
    thread::spawn(move || {
        let mut stderr = LoggedRead {
            inner: stderr,
            log,
            tag: "err",
        };
        io::copy(&mut stderr, &mut io::sink()).ok();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_log_moves_to_the_rotated_name_and_starts_fresh() {
        let dir = std::env::temp_dir().join(format!("ralph-acp-log-{}", std::process::id()));
        let log = CappedLog::new(Some(&dir), "A");
        let chunk = vec![b'x'; 600 * 1024];
        log.append("out", &chunk);
        log.append("out", &chunk);
        let path = acp_log_path(&dir, "A");
        assert!(fs::metadata(rotated_log_path(&path)).unwrap().len() > 600 * 1024);
        assert!(fs::metadata(&path).unwrap().len() <= ralph_core::worker::ACP_LOG_CAP);
        fs::remove_dir_all(&dir).ok();
    }
}
