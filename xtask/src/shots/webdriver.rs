use super::core::{Screen, Theme, Width};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
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

pub fn token_url(listening: &Listening) -> String {
    format!("{}/?token={}", listening.base, listening.token)
}

pub fn shot_url(base: &str, screen: Screen) -> String {
    match screen {
        Screen::Catalogue => format!("{base}/catalogue"),
        _ => format!("{base}/#/{}", screen.name()),
    }
}

pub fn user_js(theme: Theme, marionette_port: u16) -> String {
    let scheme = match theme {
        Theme::Dark => 0,
        Theme::Light => 1,
    };
    format!(
        "user_pref(\"layout.css.prefers-color-scheme.content-override\", {scheme});\n\
         user_pref(\"marionette.port\", {marionette_port});\n"
    )
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

pub fn parse_length(prefix: &str) -> Result<usize, WebdriverError> {
    prefix
        .parse()
        .map_err(|_| WebdriverError::BadFrameLength(prefix.to_string()))
}

pub fn command_result(message: &Value, id: u32) -> Result<Value, WebdriverError> {
    let parts = message.as_array().filter(|parts| parts.len() == 4);
    let Some([kind, got, error, result]) = parts.map(Vec::as_slice) else {
        return Err(WebdriverError::BadReply(message.to_string()));
    };
    if kind != 1 || got != id {
        return Err(WebdriverError::BadReply(message.to_string()));
    }
    if error.is_null() {
        Ok(result.clone())
    } else {
        Err(WebdriverError::CommandFailed(error.to_string()))
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

pub fn isolated_env(dir: &Path) -> Vec<(String, String)> {
    let under = |sub: &str| dir.join(sub).display().to_string();
    vec![
        ("HOME".to_string(), dir.display().to_string()),
        ("XDG_CONFIG_HOME".to_string(), under("config")),
        ("XDG_DATA_HOME".to_string(), under("data")),
        ("XDG_CACHE_HOME".to_string(), under("cache")),
        ("RALPH_CONFIG_DIR".to_string(), under("config/ralph")),
    ]
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
    fn user_js_sets_scheme_and_port() {
        let dark = user_js(Theme::Dark, 4444);
        assert!(dark.contains("prefers-color-scheme.content-override\", 0)"));
        assert!(dark.contains("marionette.port\", 4444)"));
        let light = user_js(Theme::Light, 1);
        assert!(light.contains("prefers-color-scheme.content-override\", 1)"));
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
    fn ready_scripts_check_the_expected_things() {
        let app = ready_script(Screen::Workers);
        assert!(app.contains("document.readyState === 'complete'"));
        assert!(app.contains(".r-spinner"));
        assert!(app.contains("getElementById('main')"));
        assert!(app.contains("childNodes.length > 0"));
        assert!(app.trim_start().starts_with("return "));
        let catalogue = ready_script(Screen::Catalogue);
        assert!(catalogue.contains("document.readyState === 'complete'"));
        assert!(!catalogue.contains("getElementById"));
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

    #[test]
    fn isolated_env_points_every_resolver_var_inside_the_dir() {
        let dir = Path::new("/tmp/shots-scratch/home");
        let pairs = isolated_env(dir);
        for name in [
            "HOME",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "RALPH_CONFIG_DIR",
        ] {
            let value = pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
                .unwrap_or_default();
            assert!(Path::new(value).starts_with(dir), "{name}={value}");
        }
    }

    #[test]
    fn isolated_env_names_no_real_home() {
        let pairs = isolated_env(Path::new("/tmp/shots-scratch/home"));
        assert!(!pairs.is_empty());
        assert!(pairs.iter().all(|(_, value)| !value.contains("/Users/")));
    }
}
