use super::core::{Screen, Theme, Width};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

pub const WINDOW_HEIGHT: u32 = 1200;
const APP_READY_SCRIPT: &str = "return document.readyState === 'complete' \
    && document.querySelector('.r-spinner') === null \
    && document.getElementById('main') !== null \
    && document.getElementById('main').childNodes.length > 0;";
const CATALOGUE_READY_SCRIPT: &str = "return document.readyState === 'complete';";
pub const SCHEME_SCRIPT: &str = "return window.matchMedia('(prefers-color-scheme: dark)').matches;";
const LISTENING_PREFIX: &str = "ralph web listening on http://127.0.0.1:";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WebdriverError {
    #[error("not a `ralph web listening on` line: {0}")]
    BadListeningLine(String),
    #[error("bad marionette frame length: {0}")]
    BadFrameLength(String),
    #[error("bad marionette hello: {0}")]
    BadHello(String),
    #[error("unexpected marionette reply: {0}")]
    BadReply(String),
    #[error("marionette command failed: {0}")]
    CommandFailed(String),
    #[error("the screenshot response has no base64 string")]
    NoScreenshot,
    #[error("the screenshot is not valid base64: {0}")]
    BadBase64(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listening {
    pub base: String,
    pub token: String,
}

pub fn parse_listening(line: &str) -> Result<Listening, WebdriverError> {
    let bad = || WebdriverError::BadListeningLine(line.trim().to_string());
    let rest = line.trim().strip_prefix(LISTENING_PREFIX).ok_or_else(bad)?;
    let (port, token) = rest.split_once("/?token=").ok_or_else(bad)?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) || token.is_empty() {
        return Err(bad());
    }
    Ok(Listening {
        base: format!("http://127.0.0.1:{port}"),
        token: token.to_string(),
    })
}

pub const FAILURE_STREAM_CAP: usize = 2048;

pub fn find_listening(lines: &[String]) -> Option<Listening> {
    lines.iter().find_map(|line| parse_listening(line).ok())
}

pub fn push_head(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    let room = cap.saturating_sub(buf.len());
    buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
}

pub fn push_tail(buf: &mut Vec<u8>, chunk: &[u8], cap: usize) {
    buf.extend_from_slice(chunk);
    let excess = buf.len().saturating_sub(cap);
    buf.drain(..excess);
}

pub fn push_tail_line(buf: &mut Vec<u8>, line: &str, cap: usize) {
    push_tail(buf, redact_token(line).as_bytes(), cap);
    push_tail(buf, b"\n", cap);
}

fn bounded(text: &str, cap: usize) -> &str {
    let mut end = text.len().min(cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn stream_section(name: &str, text: &str) -> String {
    let text = bounded(text.trim(), FAILURE_STREAM_CAP);
    if text.is_empty() {
        format!("no {name}")
    } else {
        format!("{name}:\n{text}")
    }
}

pub fn format_start_failure(status: &str, stdout: &str, stderr: &str) -> String {
    redact_token(&format!(
        "ralph web did not start ({})\n{}\n{}",
        bounded(status, 100),
        stream_section("stderr", stderr),
        stream_section("stdout", stdout)
    ))
}

pub fn token_url(listening: &Listening) -> String {
    format!("{}/?token={}", listening.base, listening.token)
}

pub fn shot_url(base: &str, screen: Screen) -> String {
    match screen {
        Screen::Catalogue => format!("{base}/catalogue"),
        _ => format!("{base}/#/{}", screen.name()),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConnectStep {
    Wait,
    Exited(String),
    TimedOut,
}

pub fn connect_step(exit: Option<&str>, elapsed: Duration, timeout: Duration) -> ConnectStep {
    match exit {
        Some(status) => ConnectStep::Exited(status.to_string()),
        None if elapsed >= timeout => ConnectStep::TimedOut,
        None => ConnectStep::Wait,
    }
}

pub fn parse_active_port(text: &str) -> Option<u16> {
    text.trim().parse().ok().filter(|port| *port != 0)
}

pub fn format_launch_failure(reasons: &[String]) -> String {
    let count = reasons.len();
    let launches = if count == 1 { "launch" } else { "launches" };
    let attempts: String = reasons
        .iter()
        .enumerate()
        .map(|(index, reason)| format!("\nattempt {}: {reason}", index + 1))
        .collect();
    format!("firefox did not open marionette on {count} {launches}{attempts}")
}

const TOKEN_MARK: &str = "token=";
const TOKEN_END: [char; 5] = ['&', '"', '\'', '\\', ' '];

pub fn redact_token(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(TOKEN_MARK) {
        let value = at + TOKEN_MARK.len();
        out.push_str(&rest[..value]);
        out.push_str("<redacted>");
        let after = &rest[value..];
        let end = after
            .find(|c: char| c.is_whitespace() || TOKEN_END.contains(&c))
            .unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

pub fn new_session_body() -> Value {
    json!({ "capabilities": {} })
}

pub fn screenshot_body() -> Value {
    json!({ "full": true, "hash": false })
}

pub fn command_frame(id: u32, name: &str, params: &Value) -> String {
    let message = json!([0, id, name, params]).to_string();
    format!("{}:{message}", message.len())
}

pub const MAX_LENGTH_DIGITS: usize = 9;
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

pub fn parse_length(prefix: &str) -> Result<usize, WebdriverError> {
    let bad = || WebdriverError::BadFrameLength(prefix.chars().take(20).collect());
    if prefix.is_empty()
        || prefix.len() > MAX_LENGTH_DIGITS
        || !prefix.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(bad());
    }
    match prefix.parse() {
        Ok(length) if length <= MAX_FRAME_BYTES => Ok(length),
        _ => Err(bad()),
    }
}

pub fn validate_hello(hello: &Value) -> Result<(), WebdriverError> {
    if hello["applicationType"] == "gecko" && hello["marionetteProtocol"] == 3 {
        Ok(())
    } else {
        Err(WebdriverError::BadHello(redact_token(&hello.to_string())))
    }
}

pub fn command_result(message: &Value, id: u32) -> Result<Value, WebdriverError> {
    let parts = message.as_array().filter(|parts| parts.len() == 4);
    let Some([kind, got, error, result]) = parts.map(Vec::as_slice) else {
        return Err(WebdriverError::BadReply(redact_token(&message.to_string())));
    };
    if kind != 1 || got != id {
        return Err(WebdriverError::BadReply(redact_token(&message.to_string())));
    }
    if error.is_null() {
        Ok(result.clone())
    } else {
        Err(WebdriverError::CommandFailed(redact_token(
            &error.to_string(),
        )))
    }
}

pub fn rect_body(width: Width) -> Value {
    json!({ "width": width.pixels(), "height": WINDOW_HEIGHT })
}

pub fn navigate_body(url: &str) -> Value {
    json!({ "url": url })
}

pub fn execute_body(script: &str) -> Value {
    json!({ "script": script, "args": [] })
}

pub fn ready_script(screen: Screen) -> &'static str {
    match screen {
        Screen::Catalogue => CATALOGUE_READY_SCRIPT,
        _ => APP_READY_SCRIPT,
    }
}

pub fn decode_screenshot(response: &Value) -> Result<Vec<u8>, WebdriverError> {
    let encoded = response["value"]
        .as_str()
        .ok_or(WebdriverError::NoScreenshot)?;
    STANDARD
        .decode(encoded)
        .map_err(|error| WebdriverError::BadBase64(error.to_string()))
}

pub fn scheme_matches(theme: Theme, response: &Value) -> bool {
    let dark = response["value"].as_bool();
    dark == Some(theme == Theme::Dark)
}

pub fn resolve_path(
    given: Option<PathBuf>,
    home: Option<&Path>,
    under_home: &str,
) -> Option<PathBuf> {
    given.or_else(|| home.map(|home| home.join(under_home)))
}

pub fn child_env(dir: &Path, path: Option<&str>, tmpdir: Option<&str>) -> Vec<(String, String)> {
    let under = |sub: &str| dir.join(sub).display().to_string();
    let passed = [("PATH", path), ("TMPDIR", tmpdir)]
        .into_iter()
        .filter_map(|(name, value)| Some((name.to_string(), value?.to_string())));
    passed
        .chain([
            ("HOME".to_string(), dir.display().to_string()),
            ("XDG_CONFIG_HOME".to_string(), under("config")),
            ("XDG_DATA_HOME".to_string(), under("data")),
            ("XDG_CACHE_HOME".to_string(), under("cache")),
            ("RALPH_CONFIG_DIR".to_string(), under("config/ralph")),
        ])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_listening_line() {
        let parsed =
            parse_listening("ralph web listening on http://127.0.0.1:4242/?token=abc123\n")
                .unwrap();
        assert_eq!(parsed.base, "http://127.0.0.1:4242");
        assert_eq!(parsed.token, "abc123");
    }

    #[test]
    fn rejects_bad_listening_lines() {
        for line in [
            "",
            "store: /tmp/x",
            "ralph web listening on:",
            "ralph web listening on http://127.0.0.1:4242/",
            "ralph web listening on http://127.0.0.1:4242/?token=",
            "ralph web listening on http://example.com:80/?token=a",
        ] {
            assert!(parse_listening(line).is_err(), "{line}");
        }
    }

    #[test]
    fn token_url_round_trips() {
        let listening = Listening {
            base: "http://127.0.0.1:9".into(),
            token: "t".into(),
        };
        assert_eq!(token_url(&listening), "http://127.0.0.1:9/?token=t");
    }

    #[test]
    fn every_screen_has_a_distinct_url() {
        let base = "http://127.0.0.1:9";
        assert_eq!(
            shot_url(base, Screen::Memory),
            "http://127.0.0.1:9/#/memory"
        );
        assert_eq!(
            shot_url(base, Screen::Config),
            "http://127.0.0.1:9/#/config"
        );
        assert_eq!(
            shot_url(base, Screen::Catalogue),
            "http://127.0.0.1:9/catalogue"
        );
        let mut urls: Vec<String> = Screen::THEMED
            .into_iter()
            .chain([Screen::Catalogue])
            .map(|screen| shot_url(base, screen))
            .collect();
        urls.sort();
        urls.dedup();
        assert_eq!(urls.len(), 7);
    }

    #[test]
    fn the_active_port_file_parses() {
        assert_eq!(parse_active_port("65280"), Some(65280));
        assert_eq!(parse_active_port(" 2828\n"), Some(2828));
    }

    #[test]
    fn a_bad_active_port_file_is_none() {
        for text in ["", "\n", "0", "abc", "65536", "-1", "12 34", "28x"] {
            assert_eq!(parse_active_port(text), None, "{text:?}");
        }
    }

    #[test]
    fn launch_failure_lists_every_attempt_in_order() {
        let text = format_launch_failure(&["exited (1)".into(), "timed out".into()]);
        assert_eq!(
            text,
            "firefox did not open marionette on 2 launches\nattempt 1: exited (1)\nattempt 2: timed out"
        );
    }

    #[test]
    fn launch_failure_names_one_launch_in_the_singular() {
        assert_eq!(
            format_launch_failure(&["bad hello".into()]),
            "firefox did not open marionette on 1 launch\nattempt 1: bad hello"
        );
    }

    #[test]
    fn redacts_a_token_in_the_middle_of_text() {
        assert_eq!(
            redact_token("open http://127.0.0.1:9/?token=abc123 failed"),
            "open http://127.0.0.1:9/?token=<redacted> failed"
        );
    }

    #[test]
    fn redacts_every_occurrence() {
        assert_eq!(
            redact_token("a?token=one b?token=two&x=1 c?token=three"),
            "a?token=<redacted> b?token=<redacted>&x=1 c?token=<redacted>"
        );
    }

    #[test]
    fn redacts_a_token_at_the_end_of_the_string() {
        assert_eq!(redact_token("url ?token=secret"), "url ?token=<redacted>");
        assert_eq!(redact_token("?token=secret\n"), "?token=<redacted>\n");
    }

    #[test]
    fn text_without_a_token_is_unchanged() {
        for text in ["", "no secrets here", "token without equals", "tokens=1"] {
            assert_eq!(redact_token(text), text);
        }
    }

    #[test]
    fn redacts_quoted_and_json_escaped_forms() {
        assert_eq!(
            redact_token("\"http://h/?token=abc\" and 'http://h/?token=def'"),
            "\"http://h/?token=<redacted>\" and 'http://h/?token=<redacted>'"
        );
        assert_eq!(
            redact_token("{\"url\":\"http://h/?token=abc\\\"}"),
            "{\"url\":\"http://h/?token=<redacted>\\\"}"
        );
    }

    #[test]
    fn an_empty_token_value_stays_marked() {
        assert_eq!(redact_token("?token= x"), "?token=<redacted> x");
    }

    #[test]
    fn a_redacted_marker_is_stable() {
        let once = redact_token("?token=abc");
        assert_eq!(redact_token(&once), once);
    }

    #[test]
    fn start_failures_never_show_the_token() {
        let text = format_start_failure(
            "killed",
            "ralph web listening on http://127.0.0.1:4242/?token=s3cret\n",
            "stderr ?token=other",
        );
        assert!(
            !text.contains("s3cret") && !text.contains("other"),
            "{text}"
        );
        assert!(text.contains("token=<redacted>"));
    }

    #[test]
    fn command_errors_never_show_the_token() {
        let reply = json!([1, 3, {"message": "bad url http://h/?token=s3cret"}, null]);
        let error = command_result(&reply, 3).unwrap_err().to_string();
        assert!(!error.contains("s3cret"), "{error}");
    }

    #[test]
    fn connect_waits_while_firefox_runs_before_the_deadline() {
        let step = connect_step(None, Duration::from_secs(1), Duration::from_secs(30));
        assert_eq!(step, ConnectStep::Wait);
    }

    #[test]
    fn connect_stops_at_once_when_firefox_exited() {
        let step = connect_step(
            Some("exit status: 0"),
            Duration::from_millis(100),
            Duration::from_secs(30),
        );
        assert_eq!(step, ConnectStep::Exited("exit status: 0".to_string()));
    }

    #[test]
    fn connect_times_out_at_the_deadline() {
        let timeout = Duration::from_secs(30);
        assert_eq!(connect_step(None, timeout, timeout), ConnectStep::TimedOut);
        assert_eq!(
            connect_step(None, timeout * 2, timeout),
            ConnectStep::TimedOut
        );
    }

    #[test]
    fn an_exit_wins_over_the_deadline() {
        let timeout = Duration::from_secs(30);
        assert_eq!(
            connect_step(Some("s"), timeout * 2, timeout),
            ConnectStep::Exited("s".to_string())
        );
    }

    #[test]
    fn frames_are_length_prefixed() {
        let frame = command_frame(7, "WebDriver:Navigate", &json!({"url": "u"}));
        let (length, message) = frame.split_once(':').unwrap();
        assert_eq!(parse_length(length).unwrap(), message.len());
        assert_eq!(
            serde_json::from_str::<Value>(message).unwrap(),
            json!([0, 7, "WebDriver:Navigate", {"url": "u"}])
        );
        assert!(parse_length("x").is_err());
        assert!(parse_length("").is_err());
    }

    #[test]
    fn lengths_up_to_the_cap_parse() {
        assert_eq!(parse_length("0").unwrap(), 0);
        assert_eq!(parse_length("2600000").unwrap(), 2_600_000);
        assert_eq!(
            parse_length(&MAX_FRAME_BYTES.to_string()).unwrap(),
            MAX_FRAME_BYTES
        );
    }

    #[test]
    fn huge_lengths_are_rejected() {
        assert!(parse_length(&(MAX_FRAME_BYTES + 1).to_string()).is_err());
        assert!(parse_length("999999999").is_err());
        assert!(parse_length("1000000000").is_err());
        assert!(parse_length(&"9".repeat(40)).is_err());
    }

    #[test]
    fn non_digit_lengths_are_rejected() {
        for prefix in ["+5", "-5", "5 ", " 5", "0x10", "1e3", "٣"] {
            assert!(parse_length(prefix).is_err(), "{prefix}");
        }
    }

    #[test]
    fn the_real_hello_is_accepted() {
        let hello = json!({"applicationType": "gecko", "marionetteProtocol": 3});
        assert_eq!(validate_hello(&hello), Ok(()));
    }

    #[test]
    fn bad_hellos_are_rejected() {
        for hello in [
            json!({}),
            json!([]),
            json!({"applicationType": "gecko", "marionetteProtocol": 2}),
            json!({"applicationType": "gecko"}),
            json!({"applicationType": "chrome", "marionetteProtocol": 3}),
            json!({"applicationType": "gecko", "marionetteProtocol": "3"}),
        ] {
            assert!(
                matches!(validate_hello(&hello), Err(WebdriverError::BadHello(_))),
                "{hello}"
            );
        }
    }

    #[test]
    fn replies_map_to_results_or_errors() {
        assert_eq!(
            command_result(&json!([1, 3, null, {"value": true}]), 3).unwrap(),
            json!({"value": true})
        );
        assert!(matches!(
            command_result(&json!([1, 3, {"error": "x"}, null]), 3),
            Err(WebdriverError::CommandFailed(_))
        ));
        for message in [
            json!([1, 4, null, {}]),
            json!([2, 3, null, {}]),
            json!("x"),
            json!([1, 3]),
        ] {
            assert!(
                matches!(
                    command_result(&message, 3),
                    Err(WebdriverError::BadReply(_))
                ),
                "{message}"
            );
        }
    }

    #[test]
    fn session_and_screenshot_bodies() {
        assert_eq!(new_session_body(), json!({"capabilities": {}}));
        assert_eq!(screenshot_body(), json!({"full": true, "hash": false}));
    }

    #[test]
    fn rect_and_navigate_bodies() {
        assert_eq!(
            rect_body(Width::Mobile),
            json!({"width": 390, "height": WINDOW_HEIGHT})
        );
        assert_eq!(rect_body(Width::Desktop)["width"], 1440);
        assert_eq!(navigate_body("http://h/"), json!({"url": "http://h/"}));
        assert_eq!(
            execute_body("return 1"),
            json!({"script": "return 1", "args": []})
        );
    }

    #[test]
    fn only_the_catalogue_has_its_own_ready_script() {
        let app = ready_script(Screen::Workers);
        assert_ne!(ready_script(Screen::Catalogue), app);
        for screen in Screen::THEMED {
            assert_eq!(ready_script(screen), app, "{screen:?}");
        }
    }

    #[test]
    fn decodes_the_screenshot_value() {
        let bytes = decode_screenshot(&json!({"value": "iVBORw0K"})).unwrap();
        assert_eq!(bytes, vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a]);
        assert_eq!(
            decode_screenshot(&json!({"value": 3})),
            Err(WebdriverError::NoScreenshot)
        );
        assert!(matches!(
            decode_screenshot(&json!({"value": "!!!"})),
            Err(WebdriverError::BadBase64(_))
        ));
    }

    #[test]
    fn scheme_matches_the_theme() {
        assert!(scheme_matches(Theme::Dark, &json!({"value": true})));
        assert!(scheme_matches(Theme::Light, &json!({"value": false})));
        assert!(!scheme_matches(Theme::Light, &json!({"value": true})));
        assert!(!scheme_matches(Theme::Dark, &json!({"value": null})));
        assert!(SCHEME_SCRIPT.contains("prefers-color-scheme: dark"));
    }

    #[test]
    fn resolve_path_prefers_the_given_path() {
        let home = Path::new("/home/u");
        assert_eq!(
            resolve_path(Some("/x".into()), Some(home), ".a"),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(
            resolve_path(None, Some(home), ".local/bin/g"),
            Some(PathBuf::from("/home/u/.local/bin/g"))
        );
        assert_eq!(resolve_path(None, None, ".a"), None);
    }

    const ALLOWLIST: [&str; 7] = [
        "PATH",
        "TMPDIR",
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "RALPH_CONFIG_DIR",
    ];

    fn value_of<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn child_env_names_are_exactly_the_allowlist() {
        let pairs = child_env(Path::new("/tmp/s/home"), Some("/bin"), Some("/tmp/t"));
        let mut names: Vec<&str> = pairs.iter().map(|(key, _)| key.as_str()).collect();
        names.sort_unstable();
        let mut expected = ALLOWLIST.to_vec();
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[test]
    fn child_env_points_every_resolver_var_inside_the_dir() {
        let dir = Path::new("/tmp/shots-scratch/home");
        let pairs = child_env(dir, None, None);
        for name in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "RALPH_CONFIG_DIR",
        ] {
            let value = value_of(&pairs, name).unwrap_or_default();
            assert!(Path::new(value).starts_with(dir), "{name}={value}");
        }
    }

    #[test]
    fn child_env_passes_path_and_tmpdir_only_when_given() {
        let dir = Path::new("/tmp/s/home");
        let none = child_env(dir, None, None);
        assert_eq!(value_of(&none, "PATH"), None);
        assert_eq!(value_of(&none, "TMPDIR"), None);
        let both = child_env(dir, Some("/usr/bin"), Some("/var/tmp"));
        assert_eq!(value_of(&both, "PATH"), Some("/usr/bin"));
        assert_eq!(value_of(&both, "TMPDIR"), Some("/var/tmp"));
    }

    #[test]
    fn child_env_keeps_the_callers_home_out_and_passes_path_through() {
        let scratch = tempfile::tempdir().unwrap();
        let dir = scratch.path().join("home");
        let pairs = child_env(&dir, Some("/Users/someone/bin"), None);
        for (name, value) in &pairs {
            if name == "PATH" {
                assert_eq!(value, "/Users/someone/bin");
            } else {
                assert!(
                    Path::new(value).starts_with(scratch.path()),
                    "{name}={value}"
                );
                assert!(!value.contains("/Users/someone"), "{name}={value}");
            }
        }
    }

    #[test]
    fn find_listening_returns_the_first_good_line() {
        let lines = vec![
            "noise\n".to_string(),
            "ralph web listening on http://127.0.0.1:4242/?token=abc\n".to_string(),
            "store: /tmp/x\n".to_string(),
        ];
        let found = find_listening(&lines).unwrap();
        assert_eq!(found.base, "http://127.0.0.1:4242");
        assert_eq!(found.token, "abc");
    }

    #[test]
    fn find_listening_is_none_without_a_good_line() {
        assert_eq!(find_listening(&[]), None);
        assert_eq!(find_listening(&["store: /tmp/x\n".to_string()]), None);
    }

    #[test]
    fn push_head_keeps_only_the_first_cap_bytes() {
        let mut buf = Vec::new();
        push_head(&mut buf, b"abcd", 6);
        push_head(&mut buf, b"efgh", 6);
        push_head(&mut buf, b"ijkl", 6);
        assert_eq!(buf, b"abcdef");
    }

    #[test]
    fn push_head_accepts_empty_chunks() {
        let mut buf = Vec::new();
        push_head(&mut buf, b"", 4);
        assert!(buf.is_empty());
    }

    #[test]
    fn failure_text_includes_status_and_stderr() {
        let text = format_start_failure("exit status: 3", "out line", "boom happened");
        assert!(text.contains("exit status: 3"));
        assert!(text.contains("boom happened"));
        assert!(text.contains("out line"));
    }

    #[test]
    fn failure_text_is_bounded() {
        let big = "x".repeat(100_000);
        let text = format_start_failure("exit status: 1", &big, &big);
        assert!(text.len() <= 2 * FAILURE_STREAM_CAP + 200, "{}", text.len());
    }

    #[test]
    fn failure_text_bound_respects_char_boundaries() {
        let big = "é".repeat(5_000);
        let text = format_start_failure("s", &big, &big);
        assert!(text.len() <= 2 * FAILURE_STREAM_CAP + 200);
    }

    #[test]
    fn failure_text_handles_empty_streams() {
        let text = format_start_failure("exit status: 1", "", "");
        assert!(text.contains("exit status: 1"));
        assert!(text.contains("no stderr"));
        assert!(text.contains("no stdout"));
    }

    fn tail_text(lines: &[&str], cap: usize) -> String {
        let mut buf = Vec::new();
        for line in lines {
            push_tail_line(&mut buf, line, cap);
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[test]
    fn a_front_cut_inside_a_token_value_leaks_nothing() {
        let tail = tail_text(&["listening on http://127.0.0.1:9/?token=S3CR3TV4LU3"], 14);
        assert!(!tail.contains("V4LU3") && !tail.contains("R3T"), "{tail}");
    }

    #[test]
    fn a_tail_line_with_several_tokens_redacts_each() {
        let tail = tail_text(&["a?token=one b?token=two&x=1"], 200);
        assert_eq!(tail, "a?token=<redacted> b?token=<redacted>&x=1\n");
    }

    #[test]
    fn tail_lines_without_a_token_are_kept_as_written() {
        assert_eq!(tail_text(&["one", "two"], 200), "one\ntwo\n");
    }

    #[test]
    fn push_tail_keeps_only_the_last_cap_bytes() {
        let mut buf = Vec::new();
        push_tail(&mut buf, b"abcd", 6);
        push_tail(&mut buf, b"efgh", 6);
        assert_eq!(buf, b"cdefgh");
        push_tail(&mut buf, b"0123456789", 6);
        assert_eq!(buf, b"456789");
    }

    #[test]
    fn push_tail_accepts_empty_chunks_and_zero_cap() {
        let mut buf = b"ab".to_vec();
        push_tail(&mut buf, b"", 4);
        assert_eq!(buf, b"ab");
        push_tail(&mut buf, b"cd", 0);
        assert!(buf.is_empty());
    }
}
