use ralph_core::wire::live::live_row;
use ralph_core::wire::read::{parse_calls, parse_session, Calls, ReadError};
use ralph_core::wire::record::{CallRecord, SessionRecord};
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::mpsc::Receiver;

pub enum ToStore {
    Call(Box<CallRecord>),
    Stop,
}

/// Facts from `symlink_metadata`, so a symlink is not reported as a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreRootFacts {
    pub is_symlink: bool,
    pub is_dir: bool,
    pub uid: u32,
    pub mode: u32,
}

/// `None` when this user owns a non-symlink directory whose mode is `0700`.
pub fn store_root_error(path: &Path, facts: &StoreRootFacts, uid: u32) -> Option<String> {
    let path = path.display().to_string();
    if facts.is_symlink {
        return Some(format!(
            "{path} is a symlink. Remove it and create a directory you own with mode 0700: mkdir -m 700 {path}"
        ));
    }
    if !facts.is_dir {
        return Some(format!(
            "{path} is not a directory. Remove it and create one with mode 0700: mkdir -m 700 {path}"
        ));
    }
    let mut problems = Vec::new();
    let mut fixes = Vec::new();
    if facts.uid != uid {
        problems.push(format!("owned by uid {}, not {uid}", facts.uid));
        fixes.push(format!("chown {uid} {path}"));
    }
    let mode = facts.mode & 0o777;
    if mode != 0o700 {
        problems.push(format!("mode {mode:04o}, not 0700"));
        fixes.push(format!("chmod 700 {path}"));
    }
    if problems.is_empty() {
        None
    } else {
        Some(format!(
            "{path} is {}. {}",
            problems.join(" and "),
            fixes.join(" && ")
        ))
    }
}

pub fn ensure_store_root(root: &Path) -> io::Result<()> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_root(root),
        Err(error) => Err(error),
        Ok(meta) => refuse_root(root, &meta),
    }
}

fn create_root(root: &Path) -> io::Result<()> {
    match DirBuilder::new().recursive(true).mode(0o700).create(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::symlink_metadata(root).and_then(|meta| refuse_root(root, &meta))
        }
        Err(error) => Err(error),
    }
}

fn current_uid() -> u32 {
    // SAFETY: geteuid only reads the process credential and cannot fail.
    unsafe { libc::geteuid() }
}

fn refuse_root(root: &Path, meta: &fs::Metadata) -> io::Result<()> {
    let facts = StoreRootFacts {
        is_symlink: meta.file_type().is_symlink(),
        is_dir: meta.is_dir(),
        uid: meta.uid(),
        mode: meta.mode(),
    };
    match store_root_error(root, &facts, current_uid()) {
        Some(message) => Err(io::Error::other(message)),
        None => Ok(()),
    }
}

pub fn open_session(dir: &Path, session: &SessionRecord) -> io::Result<File> {
    DirBuilder::new().mode(0o700).create(dir).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::other(format!(
                "{} already exists, so ralph will not record into it",
                dir.display()
            ))
        } else {
            error
        }
    })?;
    let mut meta = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("session.json"))?;
    meta.write_all(&serde_json::to_vec_pretty(session).map_err(io::Error::other)?)?;
    meta.flush()?;
    OpenOptions::new()
        .append(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join("calls.jsonl"))
}

fn append(file: &mut File, record: &CallRecord) -> io::Result<()> {
    let mut line = serde_json::to_vec(record).map_err(io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}

pub fn write_loop(records: Receiver<ToStore>, mut file: File) {
    for message in records {
        let ToStore::Call(record) = message else {
            return;
        };
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

fn io_error(path: &Path, error: io::Error) -> ReadError {
    ReadError::Session {
        label: path.display().to_string(),
        message: error.to_string(),
    }
}

pub fn read_session(root: &Path, id: &str) -> Result<(SessionRecord, Calls), ReadError> {
    let dir = root.join(id);
    let meta_path = dir.join("session.json");
    let meta = std::fs::read_to_string(&meta_path).map_err(|error| io_error(&meta_path, error))?;
    let session = parse_session(&meta_path.display().to_string(), &meta)?;
    let calls_path = dir.join("calls.jsonl");
    let text = match std::fs::read(&calls_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(&calls_path, error)),
    };
    let calls = parse_calls(&calls_path.display().to_string(), text.as_deref())?;
    Ok((session, calls))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn facts(uid: u32, mode: u32) -> StoreRootFacts {
        StoreRootFacts {
            is_symlink: false,
            is_dir: true,
            uid,
            mode,
        }
    }

    #[test]
    fn owned_0700_directory_is_accepted() {
        let path = Path::new("/tmp/ralph-wire");
        assert_eq!(store_root_error(path, &facts(502, 0o700), 502), None);
        assert_eq!(store_root_error(path, &facts(502, 0o40700), 502), None);
    }

    #[test]
    fn loose_mode_names_chmod() {
        let path = Path::new("/tmp/llm-wire");
        let message = store_root_error(path, &facts(502, 0o755), 502).unwrap();
        assert_eq!(
            message,
            "/tmp/llm-wire is mode 0755, not 0700. chmod 700 /tmp/llm-wire"
        );
    }

    #[test]
    fn wrong_owner_and_mode_name_both_fixes() {
        let path = Path::new("/tmp/ralph-wire");
        let message = store_root_error(path, &facts(0, 0o755), 502).unwrap();
        assert!(message.contains("/tmp/ralph-wire"), "{message}");
        assert!(message.contains("owned by uid 0, not 502"), "{message}");
        assert!(message.contains("chmod 700 /tmp/ralph-wire"), "{message}");
        assert!(message.contains("chown 502 /tmp/ralph-wire"), "{message}");
    }

    #[test]
    fn symlink_and_file_are_refused_before_chmod() {
        let path = Path::new("/tmp/ralph-wire");
        let link = StoreRootFacts {
            is_symlink: true,
            is_dir: false,
            uid: 502,
            mode: 0o777,
        };
        let message = store_root_error(path, &link, 502).unwrap();
        assert!(message.contains("symlink"), "{message}");
        assert!(
            message.contains("mkdir -m 700 /tmp/ralph-wire"),
            "{message}"
        );
        assert!(!message.contains("chmod"), "{message}");
        let file = StoreRootFacts {
            is_symlink: false,
            is_dir: false,
            uid: 502,
            mode: 0o600,
        };
        let message = store_root_error(path, &file, 502).unwrap();
        assert!(message.contains("not a directory"), "{message}");
        assert!(
            message.contains("mkdir -m 700 /tmp/ralph-wire"),
            "{message}"
        );
    }
}
