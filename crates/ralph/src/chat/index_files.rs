use crate::chat::memory::{check_path, read_optional};
use ralph_core::chat_memory::{
    refresh, write_plan, ByteSource, IndexWrite, LogIndex, LOG_INDEX_FILE,
};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A store file opened once. `size` is its length at open, so lines appended
/// later stay out of this snapshot. A missing file reads as empty.
pub struct FileSource {
    file: Option<File>,
    size: u64,
}

impl FileSource {
    pub fn open(path: &Path) -> io::Result<Self> {
        check_path(path)?;
        match File::open(path) {
            Ok(file) => {
                let size = file.metadata()?.len();
                Ok(Self {
                    file: Some(file),
                    size,
                })
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self {
                file: None,
                size: 0,
            }),
            Err(e) => Err(e),
        }
    }
}

impl ByteSource for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>> {
        let len = usize::try_from(end.saturating_sub(start)).map_err(io::Error::other)?;
        let mut buf = vec![0; len];
        if len == 0 {
            return Ok(buf);
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("chat store file vanished"))?;
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

fn stored_log_index(dir: &Path) -> io::Result<Option<LogIndex>> {
    let bytes = read_optional(&dir.join(LOG_INDEX_FILE))?;
    Ok(LogIndex::decode(&bytes))
}

/// The log and its index, brought up to date in memory only. The caller holds
/// a shared `LOCK` so no append is half written while the length is taken.
pub fn snapshot_log(dir: &Path) -> io::Result<(FileSource, LogIndex)> {
    let mut log = FileSource::open(&dir.join("log.jsonl"))?;
    let (index, _) = refresh(stored_log_index(dir)?, &mut log)?;
    Ok((log, index))
}

/// Brings `log.idx` up to date on disk. The caller holds the exclusive `LOCK`.
pub fn update_log_index(dir: &Path) -> io::Result<()> {
    let mut log = FileSource::open(&dir.join("log.jsonl"))?;
    let (index, how) = refresh(stored_log_index(dir)?, &mut log)?;
    persist(dir, &dir.join(LOG_INDEX_FILE), write_plan(&index, how))
}

fn persist(dir: &Path, path: &Path, write: IndexWrite) -> io::Result<()> {
    check_path(path)?;
    match write {
        IndexWrite::Keep => Ok(()),
        IndexWrite::InPlace { at, body, header } => {
            let mut file = OpenOptions::new().write(true).open(path)?;
            file.seek(SeekFrom::Start(at))?;
            file.write_all(&body)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&header)
        }
        IndexWrite::Replace(bytes) => {
            let mut temp = tempfile::NamedTempFile::new_in(dir)?;
            temp.write_all(&bytes)?;
            temp.persist(path).map_err(|e| e.error)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn file_source_keeps_its_open_length_and_reads_missing_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        assert_eq!(FileSource::open(&path).unwrap().size(), 0);
        fs::write(&path, "abcdef\n").unwrap();
        let mut source = FileSource::open(&path).unwrap();
        fs::write(&path, "abcdef\nmore\n").unwrap();
        assert_eq!(source.size(), 7);
        assert_eq!(source.read(2, 5).unwrap(), b"cde");
        assert!(source.read(3, 3).unwrap().is_empty());
    }

    #[test]
    fn persist_writes_in_place_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_INDEX_FILE);
        persist(
            dir.path(),
            &path,
            IndexWrite::Replace(b"0123456789".to_vec()),
        )
        .unwrap();
        let in_place = IndexWrite::InPlace {
            at: 10,
            body: b"ab".to_vec(),
            header: b"HH".to_vec(),
        };
        persist(dir.path(), &path, in_place).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"HH23456789ab");
        persist(dir.path(), &path, IndexWrite::Keep).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"HH23456789ab");
    }

    #[cfg(unix)]
    #[test]
    fn persist_rejects_a_symlinked_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_INDEX_FILE);
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), &path).unwrap();
        assert!(persist(dir.path(), &path, IndexWrite::Replace(Vec::new())).is_err());
    }
}
