use std::path::{Path, PathBuf};

pub const ACP_LOG_DIR: &str = "acp-logs";
pub const ACP_LOG_CAP: u64 = 1 << 20;

pub fn acp_log_path(store: &Path, id: &str) -> PathBuf {
    store.join(ACP_LOG_DIR).join(format!("{id}.log"))
}

pub fn rotated_log_path(log: &Path) -> PathBuf {
    let mut name = log.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

pub fn rotates(len: u64, incoming: u64) -> bool {
    len > 0 && len + incoming > ACP_LOG_CAP
}

pub fn log_entry(at: u64, tag: &str, chunk: &[u8]) -> Vec<u8> {
    let mut entry = format!("{at} {tag} ").into_bytes();
    entry.extend_from_slice(chunk);
    if !chunk.ends_with(b"\n") {
        entry.push(b'\n');
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_waits_for_a_non_empty_log_that_would_pass_the_cap() {
        assert!(!rotates(0, ACP_LOG_CAP + 1));
        assert!(!rotates(10, ACP_LOG_CAP - 10));
        assert!(rotates(10, ACP_LOG_CAP - 9));
    }

    #[test]
    fn entries_end_with_one_newline() {
        assert_eq!(log_entry(5, "out", b"{}"), b"5 out {}\n");
        assert_eq!(log_entry(5, "err", b"x\n"), b"5 err x\n");
    }

    #[test]
    fn the_rotated_log_keeps_the_log_name() {
        let log = acp_log_path(Path::new("/s"), "A");
        assert_eq!(rotated_log_path(&log), Path::new("/s/acp-logs/A.log.1"));
    }
}
