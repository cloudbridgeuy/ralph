use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

struct State {
    file: File,
    pending: HashSet<String>,
    error: Option<String>,
}

#[derive(Clone)]
pub(super) struct Evidence(Arc<Mutex<State>>);

impl Evidence {
    pub(super) fn create() -> io::Result<(Self, PathBuf)> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("ralph-agy-evidence-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let dir = builder.tempdir()?;
        let path = dir.path().join("permissions.jsonl");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        let path = dir.keep().join("permissions.jsonl");
        Ok((
            Self(Arc::new(Mutex::new(State {
                file,
                pending: HashSet::new(),
                error: None,
            }))),
            path,
        ))
    }

    fn record(&self, direction: &'static str, frame: &[u8]) -> io::Result<()> {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(frame) else {
            return Ok(());
        };
        let Some(id) = value.get("id") else {
            return Ok(());
        };
        let key = id.to_string();
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("evidence lock poisoned"))?;
        let relevant = if direction == "agent_to_client" {
            if value["method"] == "session/request_permission" {
                state.pending.insert(key);
                true
            } else {
                false
            }
        } else {
            value.get("method").is_none() && state.pending.remove(&key)
        };
        if relevant {
            let raw = std::str::from_utf8(frame).map_err(io::Error::other)?;
            let entry = serde_json::json!({"direction": direction, "frame": raw});
            let result = writeln!(state.file, "{entry}");
            if let Err(error) = result {
                state.error = Some(error.to_string());
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn error(&self) -> Option<String> {
        match self.0.lock() {
            Ok(state) => state.error.clone(),
            Err(_) => Some("evidence lock poisoned".into()),
        }
    }
}

pub(super) struct Capture<T> {
    inner: T,
    evidence: Option<Evidence>,
    buffer: Vec<u8>,
    direction: &'static str,
}

impl<T> Capture<T> {
    pub(super) fn new(inner: T, evidence: Option<Evidence>, direction: &'static str) -> Self {
        Self {
            inner,
            evidence,
            buffer: Vec::new(),
            direction,
        }
    }

    fn capture(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some(evidence) = &self.evidence else {
            return Ok(());
        };
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            evidence.record(self.direction, &self.buffer[..end])?;
            self.buffer.drain(..=end);
        }
        Ok(())
    }
}

impl<T: Read> Read for Capture<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.inner.read(buf)?;
        self.capture(&buf[..count])?;
        Ok(count)
    }
}

impl<T: Write> Write for Capture<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(buf)?;
        self.capture(&buf[..count])?;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_only_permission_frames_and_matching_written_replies(
    ) -> Result<(), Box<dyn std::error::Error>> {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
        let (evidence, path) = Evidence::create()?;
        let _cleanup = Cleanup(
            path.parent()
                .ok_or("missing evidence directory")?
                .to_path_buf(),
        );
        assert!(path.is_file());
        assert!(std::fs::read(&path)?.is_empty());
        let mut input = Capture::new(&b"{\"id\":1,\"method\":\"other\"}\n{\"id\":17,\"method\":\"session/request_permission\",\"params\":{\"vendor\":true}}\n{\"id\":18,\"method\":\"session/request_permission\",\"params\":{}}\n"[..], Some(evidence.clone()), "agent_to_client");
        let mut data = Vec::new();
        input.read_to_end(&mut data)?;
        let mut output = Capture::new(Vec::new(), Some(evidence), "client_to_agent");
        output.write_all(b"{\"id\":1,\"result\":{}}\n{\"id\":18,\"error\":{}}\n{\"id\":17,")?;
        output.write_all(b"\"result\":{\"outcome\":\"cancelled\"}}\n")?;
        drop(input);
        drop(output);
        let text = std::fs::read_to_string(path)?;
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0]["direction"], "agent_to_client");
        assert_eq!(rows[1]["direction"], "agent_to_client");
        assert_eq!(rows[2]["direction"], "client_to_agent");
        assert_eq!(rows[3]["direction"], "client_to_agent");
        let request: serde_json::Value =
            serde_json::from_str(rows[0]["frame"].as_str().ok_or("missing frame")?)?;
        let reply: serde_json::Value =
            serde_json::from_str(rows[3]["frame"].as_str().ok_or("missing frame")?)?;
        assert_eq!(request["id"], reply["id"]);
        assert_eq!(request["params"]["vendor"], true);
        Ok(())
    }
    #[test]
    fn evidence_write_error_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("read-only");
        File::create(&path)?;
        let evidence = Evidence(Arc::new(Mutex::new(State {
            file: File::open(path)?,
            pending: HashSet::new(),
            error: None,
        })));
        assert!(evidence
            .record(
                "agent_to_client",
                br#"{"id":1,"method":"session/request_permission"}"#
            )
            .is_err());
        assert!(evidence.error().is_some());
        Ok(())
    }
}
