use crate::chat::memory::{check_path, read_optional};
use ralph_core::chat_memory::{
    index_temp_prefix, is_index_temp, refresh, write_plan, Archive, ByteSource, Index, IndexWrite,
    NodeSlot, Slot, INDEX_TEMP_RAND, INDEX_TEMP_SUFFIX, LOG_INDEX_FILE, TREE_INDEX_FILE,
};
use std::fs::{self, File, OpenOptions};
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

    pub fn exists(&self) -> bool {
        self.file.is_some()
    }
}

impl ByteSource for FileSource {
    fn size(&self) -> u64 {
        self.size
    }

    fn read(&mut self, start: u64, end: u64) -> io::Result<Vec<u8>> {
        let len = usize::try_from(end.saturating_sub(start)).map_err(io::Error::other)?;
        if end > self.size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "read past the chat store snapshot",
            ));
        }
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

fn stored<S: Slot>(dir: &Path) -> io::Result<Option<Index<S>>> {
    Ok(Index::decode(&read_optional(&dir.join(S::INDEX_FILE))?))
}

/// The log, the tree and their indexes, brought up to date in memory. The
/// caller holds a shared `LOCK` so no append is half written meanwhile.
pub fn open_archive(dir: &Path) -> io::Result<Archive<FileSource>> {
    let log = FileSource::open(&dir.join(<u64 as Slot>::FILE))?;
    let tree = FileSource::open(&dir.join(NodeSlot::FILE))?;
    Archive::open(log, stored(dir)?, tree, stored(dir)?)
}

/// Writes each index a reader had to extend or rebuild, so the next reader
/// does not repeat the work. The caller holds a shared `LOCK`: no writer runs,
/// and every reader that holds it sees the same file lengths, so two readers
/// write the same bytes. Each write is a whole file renamed into place, so a
/// concurrent reader sees the old index or the new one, never a mix. An index
/// for a missing file is not written.
pub fn save_stale(dir: &Path, archive: &Archive<FileSource>) -> io::Result<()> {
    if archive.stale.log && archive.log.exists() {
        let bytes = archive.log_index.encode();
        persist(dir, &dir.join(LOG_INDEX_FILE), IndexWrite::Replace(bytes))?;
    }
    if archive.stale.tree && archive.tree.file.exists() {
        let bytes = archive.tree.index.encode();
        persist(dir, &dir.join(TREE_INDEX_FILE), IndexWrite::Replace(bytes))?;
    }
    Ok(())
}

/// Brings the index of `S::FILE` up to date on disk. The caller holds the
/// exclusive `LOCK`.
pub fn update_index<S: Slot>(dir: &Path) -> io::Result<()> {
    let mut file = FileSource::open(&dir.join(S::FILE))?;
    let (index, how) = refresh(stored::<S>(dir)?, &mut file)?;
    persist(dir, &dir.join(S::INDEX_FILE), write_plan(&index, how))
}

/// Deletes the index temp files a killed save left behind. The caller holds
/// the exclusive `LOCK`, so no save is running and every match is orphaned.
pub fn sweep_index_temps(dir: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let temp = entry.file_name().to_str().is_some_and(is_index_temp);
        if temp && entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// The temp file `path` is saved through, named so `is_index_temp` matches it.
pub fn index_temp(dir: &Path, path: &Path) -> io::Result<tempfile::NamedTempFile> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("chat index path has no file name"))?;
    tempfile::Builder::new()
        .prefix(&index_temp_prefix(name))
        .suffix(INDEX_TEMP_SUFFIX)
        .rand_bytes(INDEX_TEMP_RAND)
        .tempfile_in(dir)
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
            let mut temp = index_temp(dir, path)?;
            temp.write_all(&bytes)?;
            temp.persist(path).map_err(|e| e.error)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
