use crate::worker::{Report, ReportStatus};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RECONNECT_LIMIT: u32 = 5;
pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);
pub const SILENCE_TIMEOUT: Duration = Duration::from_secs(45);
pub const OPENCODE_SESSION_DIR: &str = "opencode-session";
pub const OPENCODE_SERVER_LOG: &str = "opencode-server.log";
pub const OPENCODE_PASSWORD_ENV: &str = "OPENCODE_PASSWORD";
const SERVER_HOST: &str = "127.0.0.1";
const SERVER_USER: &str = "opencode";
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn reconnect_delay(attempt: u32) -> Option<Duration> {
    (attempt < RECONNECT_LIMIT).then_some(RECONNECT_DELAY)
}

pub fn opencode_session_path(store: &Path, id: &str) -> PathBuf {
    store.join(OPENCODE_SESSION_DIR).join(id)
}

pub fn opencode_server_log(store: &Path) -> PathBuf {
    store.join(OPENCODE_SERVER_LOG)
}

pub fn server_url(port: u16) -> String {
    format!("http://{SERVER_HOST}:{port}")
}

pub fn opencode_serve_command(port: u16) -> Vec<String> {
    [
        "zsh",
        "-lic",
        r#"exec "$0" "$@""#,
        "opencode",
        "serve",
        "--hostname",
        SERVER_HOST,
        "--port",
        &port.to_string(),
    ]
    .map(str::to_string)
    .to_vec()
}

fn base64(bytes: &[u8]) -> String {
    bytes
        .chunks(3)
        .flat_map(|chunk| {
            let word = chunk.iter().enumerate().fold(0u32, |word, (at, byte)| {
                word | u32::from(*byte) << (16 - 8 * at)
            });
            (0..4).map(move |at| {
                if at <= chunk.len() {
                    char::from(BASE64_ALPHABET[((word >> (18 - 6 * at)) & 63) as usize])
                } else {
                    '='
                }
            })
        })
        .collect()
}

pub fn basic_auth(password: &str) -> String {
    format!(
        "Basic {}",
        base64(format!("{SERVER_USER}:{password}").as_bytes())
    )
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn parse_ps(output: &str) -> Vec<(u32, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            Some((pid, parent))
        })
        .collect()
}

pub fn descendants(rows: &[(u32, u32)], root: u32) -> Vec<u32> {
    let mut found = vec![root];
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        next += 1;
        for (pid, ppid) in rows {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
    }
    found.remove(0);
    found
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpencodeRecord {
    pub session_id: String,
    pub cwd: PathBuf,
    pub model: String,
    pub inbox: PathBuf,
}

impl OpencodeRecord {
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

pub fn opencode_failure_report(id: &str, at: &str, problem: &str) -> Report {
    Report {
        worker: id.to_string(),
        at: at.to_string(),
        status: ReportStatus::Blocked.as_str().to_string(),
        text: format!("The opencode turn failed: {problem}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PS_OUTPUT: &str = "    1     0\n  100     1\n  200   100\n  300   200\n  301   200\n  400     1\n  500   300\nbogus line\n";
    use crate::worker::SERVER_STOPPED;

    #[test]
    fn reconnect_delay_allows_five_attempts_two_seconds_apart() {
        for attempt in 0..RECONNECT_LIMIT {
            assert_eq!(reconnect_delay(attempt), Some(Duration::from_secs(2)));
        }
        assert_eq!(reconnect_delay(RECONNECT_LIMIT), None);
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(
            base64(b"Aladdin:open sesame"),
            "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    #[test]
    fn basic_auth_uses_the_opencode_user() {
        assert_eq!(basic_auth("s3cret"), "Basic b3BlbmNvZGU6czNjcmV0");
    }

    #[test]
    fn hex_encode_pads_each_byte() {
        assert_eq!(hex_encode(&[0, 15, 16, 255]), "000f10ff");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn serve_command_runs_opencode_through_the_login_shell() {
        let args = opencode_serve_command(4096);
        assert_eq!(args[..4], ["zsh", "-lic", r#"exec "$0" "$@""#, "opencode"]);
        assert_eq!(
            args[4..],
            ["serve", "--hostname", "127.0.0.1", "--port", "4096"]
        );
        assert_eq!(server_url(4096), "http://127.0.0.1:4096");
    }

    #[test]
    fn paths_live_under_the_store() {
        let store = Path::new("/s");
        assert_eq!(
            opencode_session_path(store, "GUZ-1"),
            Path::new("/s/opencode-session/GUZ-1")
        );
        assert_eq!(
            opencode_server_log(store),
            Path::new("/s/opencode-server.log")
        );
    }

    #[test]
    fn parse_ps_reads_pid_and_parent_rows() {
        let rows = parse_ps(PS_OUTPUT);
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[2], (200, 100));
        assert_eq!(parse_ps(""), Vec::new());
    }

    #[test]
    fn descendants_walk_the_whole_tree_without_the_root() {
        let rows = parse_ps(PS_OUTPUT);
        assert_eq!(descendants(&rows, 100), vec![200, 300, 301, 500]);
        assert_eq!(descendants(&rows, 301), Vec::<u32>::new());
        assert_eq!(descendants(&rows, 999), Vec::<u32>::new());
    }

    #[test]
    fn descendants_survive_a_parent_cycle() {
        let rows = vec![(2, 1), (3, 2), (2, 3)];
        assert_eq!(descendants(&rows, 1), vec![2, 3]);
    }

    #[test]
    fn record_round_trips_and_rejects_garbage() {
        let record = OpencodeRecord {
            session_id: "ses_one".to_string(),
            cwd: PathBuf::from("/w"),
            model: "github-copilot/claude-haiku-4.5".to_string(),
            inbox: PathBuf::from("/i"),
        };
        assert_eq!(OpencodeRecord::parse(&record.render()), Some(record));
        assert_eq!(OpencodeRecord::parse("not json"), None);
    }

    #[test]
    fn failure_report_blocks_with_the_problem() {
        let report = opencode_failure_report("GUZ-1", "now", SERVER_STOPPED);
        assert_eq!(report.worker, "GUZ-1");
        assert_eq!(report.status, ReportStatus::Blocked.as_str());
        assert_eq!(
            report.text,
            "The opencode turn failed: the opencode server stopped during the turn."
        );
    }
}
