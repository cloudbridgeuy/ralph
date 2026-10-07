use crate::invoke::build_memory_command;
use crate::iteration::extract_response_text;
use crate::meeting::MeetingConfig;
use crate::spinner::SpinnerSessionInfo;
use crate::stream_processor::VerboseToolsConfig;
use crate::subprocess::{invoke_subprocess_with_spinner_config, SpinnerSubprocessConfig};
use crate::warn::warn;
use ralph_core::meeting_memory::{nap_summary, normalize_note, snapshot, Snapshot, Wake};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

pub struct Memory {
    dir: PathBuf,
}

fn read_optional(path: PathBuf) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

impl Memory {
    pub fn new(project: PathBuf) -> Self {
        Self {
            dir: project.join(".ralph/memory"),
        }
    }

    fn check_paths(&self) -> io::Result<()> {
        for path in [
            self.dir.parent().map(PathBuf::from),
            Some(self.dir.clone()),
            Some(self.dir.join("LOCK")),
            Some(self.dir.join("LOG.txt")),
            Some(self.dir.join("WAKE.json")),
        ]
        .into_iter()
        .flatten()
        {
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(io::Error::other(format!(
                        "office memory path is a symlink: {}",
                        path.display()
                    )));
                }
                #[cfg(unix)]
                Ok(metadata) if metadata.is_file() && metadata.nlink() > 1 => {
                    return Err(io::Error::other(format!(
                        "office memory file has multiple links: {}",
                        path.display()
                    )));
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        if let Some(home) = dirs::home_dir() {
            let protected = home.join(".optmem");
            let protected = protected.canonicalize().unwrap_or(protected);
            if self
                .dir
                .ancestors()
                .filter_map(|p| p.canonicalize().ok())
                .any(|p| p.starts_with(&protected))
            {
                return Err(io::Error::other(
                    "office memory cannot use personal memory paths",
                ));
            }
        }
        Ok(())
    }

    fn lock(&self) -> io::Result<File> {
        self.check_paths()?;
        fs::create_dir_all(&self.dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("LOCK"))?;
        lock.lock()?;
        Ok(lock)
    }

    fn load(&self) -> io::Result<(Snapshot, Option<Vec<u8>>)> {
        let log = read_optional(self.dir.join("LOG.txt"))?.unwrap_or_default();
        let state = read_optional(self.dir.join("WAKE.json"))?;
        let loaded = snapshot(&log, state.as_deref());
        for warning in &loaded.warnings {
            warn(warning);
        }
        Ok((loaded, state))
    }

    pub fn wake(&self) -> String {
        let loaded = (|| {
            let _lock = self.lock()?;
            self.load()
        })();
        match loaded {
            Ok((loaded, _)) => loaded.context(),
            Err(e) => {
                warn(format!("could not read office memory: {e}"));
                let log = self
                    .check_paths()
                    .and_then(|()| read_optional(self.dir.join("LOG.txt")))
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                snapshot(&log, None).context()
            }
        }
    }

    pub fn append(&self, text: &str) -> io::Result<()> {
        let note = normalize_note(text).map_err(io::Error::other)?;
        let _lock = self.lock()?;
        let log = read_optional(self.dir.join("LOG.txt"))?.unwrap_or_default();
        let prefix = if log.is_empty() || log.ends_with(b"\n") {
            ""
        } else {
            "\n"
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("LOG.txt"))?;
        file.write_all(format!("{prefix}{note}\n").as_bytes())?;
        file.sync_all()
    }

    fn compress(
        &self,
        mut curate: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (loaded, prior) = {
            let _lock = self.lock()?;
            self.load()?
        };
        if loaded.pending.is_empty() || !loaded.compressible {
            return Ok(());
        }
        let summary = curate(&loaded.nap_prompt()).map_err(io::Error::other)?;
        let summary = nap_summary(0, Some(&summary)).map_err(io::Error::other)?;
        let _lock = self.lock()?;
        if read_optional(self.dir.join("WAKE.json"))? != prior {
            return Err(
                io::Error::other("office memory changed during nap; retry next meeting").into(),
            );
        }
        let wake = Wake {
            summary,
            cursor: loaded.count,
        };
        let mut temp = tempfile::NamedTempFile::new_in(&self.dir)?;
        serde_json::to_writer(&mut temp, &wake)?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(self.dir.join("WAKE.json"))?;
        Ok(())
    }

    pub fn nap(&self, config: &MeetingConfig) {
        let result = self.compress(|prompt| {
            let command = build_memory_command(prompt, dirs::home_dir().as_deref());
            let result = invoke_subprocess_with_spinner_config(&SpinnerSubprocessConfig {
                command,
                timeout_secs: config.timeout_secs,
                theme_config: config.theme_config.clone(),
                session_elapsed_ms: 0,
                verbose_tools: VerboseToolsConfig::new(),
                session_info: SpinnerSessionInfo {
                    persona: Some("office memory".into()),
                    slug: None,
                    current_iteration: None,
                    max_iterations: None,
                    on_behalf_of: None,
                },
                pause: None,
                cancel: None,
            })
            .map_err(|e| e.to_string())?;
            let response = extract_response_text(&result.stream_result.output_blocks);
            nap_summary(result.exit_code, response.as_deref())
        });
        if let Err(e) = result {
            warn(format!("office memory nap failed; notes retained: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_incremental_nap_and_failures_preserve_log_and_wake() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path().to_path_buf());
        memory.append("  first\n note ").unwrap();
        assert!(memory.wake().contains("first note"));
        memory
            .compress(|prompt| {
                assert!(prompt.contains("first note"));
                Ok("summary".into())
            })
            .unwrap();
        let prior = fs::read(memory.dir.join("WAKE.json")).unwrap();
        memory.append("second").unwrap();
        for response in [Err("offline".into()), Ok("  ".into())] {
            assert!(memory.compress(|_| response.clone()).is_err());
            assert_eq!(fs::read(memory.dir.join("WAKE.json")).unwrap(), prior);
        }
        assert_eq!(
            fs::read_to_string(memory.dir.join("LOG.txt")).unwrap(),
            "first note\nsecond\n"
        );
        let wake: serde_json::Value = serde_json::from_str(&memory.wake()).unwrap();
        assert_eq!(wake["summary"], "summary");
        assert_eq!(wake["pending_notes"], serde_json::json!(["second"]));
        memory
            .compress(|_| {
                memory.append("third").unwrap();
                Ok("merged".into())
            })
            .unwrap();
        assert!(memory.wake().contains("third"));
        memory.compress(|_| Ok("done".into())).unwrap();
        memory.compress(|_| panic!("no pending notes")).unwrap();
    }

    #[test]
    fn corrupt_wake_replays_notes_and_append_errors_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path().to_path_buf());
        memory.append("durable").unwrap();
        fs::write(memory.dir.join("WAKE.json"), b"corrupt").unwrap();
        assert!(memory.wake().contains("durable"));
        fs::remove_file(memory.dir.join("WAKE.json")).unwrap();
        fs::create_dir(memory.dir.join("WAKE.json")).unwrap();
        assert!(memory.wake().contains("durable"));
        assert!(memory.compress(|_| panic!("read failure")).is_err());
        fs::remove_file(memory.dir.join("LOG.txt")).unwrap();
        fs::create_dir(memory.dir.join("LOG.txt")).unwrap();
        assert!(memory.append("new").is_err());
        assert!(memory.append(" \n").is_err());
    }

    #[test]
    fn competing_nap_does_not_overwrite_newer_summary() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Memory::new(dir.path().to_path_buf());
        memory.append("first").unwrap();
        let result = memory.compress(|_| {
            memory.compress(|_| Ok("newer summary".into())).unwrap();
            memory.append("second").unwrap();
            Ok("stale summary".into())
        });
        assert!(result.is_err());
        let wake: serde_json::Value = serde_json::from_str(&memory.wake()).unwrap();
        assert_eq!(wake["summary"], "newer summary");
        assert_eq!(wake["pending_notes"], serde_json::json!(["second"]));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_memory_paths_never_read_or_write_external_data() {
        use std::os::unix::fs::symlink;

        for name in [
            ".ralph",
            ".ralph/memory",
            ".ralph/memory/LOG.txt",
            ".ralph/memory/WAKE.json",
            ".ralph/memory/LOCK",
        ] {
            let project = tempfile::tempdir().unwrap();
            let external = tempfile::tempdir().unwrap();
            let target =
                if name.ends_with(".txt") || name.ends_with(".json") || name.ends_with("LOCK") {
                    let target = external.path().join("private");
                    fs::write(&target, "private knowledge\n").unwrap();
                    target
                } else {
                    external.path().to_path_buf()
                };
            let link = project.path().join(name);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink(&target, &link).unwrap();
            let memory = Memory::new(project.path().to_path_buf());
            assert!(memory.append("new").is_err());
            assert!(!memory.wake().contains("private knowledge"));
            assert!(memory.compress(|_| panic!("unsafe paths")).is_err());
            if target.is_file() {
                assert_eq!(fs::read_to_string(target).unwrap(), "private knowledge\n");
            } else {
                assert_eq!(fs::read_dir(target).unwrap().count(), 0);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn hardlinked_log_never_changes_external_data() {
        let project = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        fs::write(external.path(), "private knowledge\n").unwrap();
        let memory = Memory::new(project.path().to_path_buf());
        fs::create_dir_all(&memory.dir).unwrap();
        fs::hard_link(external.path(), memory.dir.join("LOG.txt")).unwrap();
        assert!(memory.append("new").is_err());
        assert!(!memory.wake().contains("private knowledge"));
        assert!(memory.compress(|_| panic!("unsafe paths")).is_err());
        assert_eq!(
            fs::read_to_string(external.path()).unwrap(),
            "private knowledge\n"
        );
    }
}
