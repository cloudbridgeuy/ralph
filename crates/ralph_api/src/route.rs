use crate::token::{extract_tokens, Token};
use std::path::PathBuf;

const HEALTH_PATH: &str = "/api/health";
const JSON: &str = "application/json";
const NO_STORE: (&str, &str) = ("Cache-Control", "no-store");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub host: Option<String>,
    pub authorization: Option<String>,
    pub cookies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    pub store: PathBuf,
    pub allowed_hosts: Vec<String>,
    pub version: String,
}

pub fn route(request: &ApiRequest, config: &ApiConfig, token: &Token) -> ApiResponse {
    if !host_allowed(request.host.as_deref(), &config.allowed_hosts) {
        return error(403, "forbidden host");
    }
    if !authorized(request, token) {
        return error(401, "unauthorized");
    }
    match (request.path.as_str(), request.method.as_str()) {
        (HEALTH_PATH, "GET") => health(&config.version),
        (HEALTH_PATH, _) => with_header(error(405, "method not allowed"), ("Allow", "GET")),
        _ => error(404, "not found"),
    }
}

pub fn overloaded() -> ApiResponse {
    error(503, "busy")
}

fn host_allowed(host: Option<&str>, allowed: &[String]) -> bool {
    host.is_some_and(|host| allowed.iter().any(|name| name.eq_ignore_ascii_case(host)))
}

fn authorized(request: &ApiRequest, token: &Token) -> bool {
    let cookies: Vec<&str> = request.cookies.iter().map(String::as_str).collect();
    extract_tokens(request.authorization.as_deref(), &request.query, &cookies)
        .iter()
        .any(|candidate| token.matches(candidate))
}

fn health(version: &str) -> ApiResponse {
    json_response(200, serde_json::json!({"status": "ok", "version": version}))
}

fn error(status: u16, message: &str) -> ApiResponse {
    json_response(status, serde_json::json!({ "error": message }))
}

fn json_response(status: u16, body: serde_json::Value) -> ApiResponse {
    ApiResponse {
        status,
        content_type: JSON,
        headers: vec![NO_STORE],
        body: body.to_string(),
    }
}

fn with_header(mut response: ApiResponse, header: (&'static str, &'static str)) -> ApiResponse {
    response.headers.push(header);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    const TOKEN: &str = "t0k3n";

    fn token() -> Token {
        Token::from_value(TOKEN)
    }

    fn config() -> ApiConfig {
        ApiConfig {
            store: PathBuf::from("/secret/store"),
            allowed_hosts: vec!["h.test:1".to_string(), "alias.test:1".to_string()],
            version: "9.9.9".to_string(),
        }
    }

    fn request(method: &str, path: &str, query: &str) -> ApiRequest {
        ApiRequest {
            method: method.to_string(),
            path: path.to_string(),
            query: query.to_string(),
            host: Some("h.test:1".to_string()),
            authorization: None,
            cookies: vec![],
        }
    }

    fn authed(method: &str, path: &str) -> ApiRequest {
        request(method, path, &format!("token={TOKEN}"))
    }

    fn body(response: &ApiResponse) -> Value {
        serde_json::from_str(&response.body).unwrap()
    }

    #[test]
    fn health_with_token_is_200_json_ok() {
        let response = route(&authed("GET", "/api/health"), &config(), &token());
        assert_eq!(response.status, 200);
        assert_eq!(body(&response), json!({"status": "ok", "version": "9.9.9"}));
        assert!(!response.body.contains("/secret/store"));
    }

    #[test]
    fn health_without_token_is_401() {
        let response = route(&request("GET", "/api/health", ""), &config(), &token());
        assert_eq!(response.status, 401);
        assert_eq!(body(&response), json!({"error": "unauthorized"}));
    }

    #[test]
    fn health_with_wrong_token_is_401() {
        let response = route(
            &request("GET", "/api/health", "token=nope"),
            &config(),
            &token(),
        );
        assert_eq!(response.status, 401);
    }

    #[test]
    fn unknown_path_with_token_is_404() {
        let response = route(&authed("GET", "/api/other"), &config(), &token());
        assert_eq!(response.status, 404);
        assert_eq!(body(&response), json!({"error": "not found"}));
    }

    #[test]
    fn unknown_path_without_token_is_401() {
        let response = route(&request("GET", "/api/other", ""), &config(), &token());
        assert_eq!(response.status, 401);
    }

    #[test]
    fn post_health_is_405_and_allow_get() {
        let response = route(&authed("POST", "/api/health"), &config(), &token());
        assert_eq!(response.status, 405);
        assert!(response.headers.contains(&("Allow", "GET")));
    }

    #[test]
    fn bad_host_is_403() {
        let mut bad = authed("GET", "/api/health");
        bad.host = Some("evil.example".to_string());
        let response = route(&bad, &config(), &token());
        assert_eq!(response.status, 403);
        assert_eq!(body(&response), json!({"error": "forbidden host"}));
    }

    #[test]
    fn missing_host_is_403() {
        let mut bad = authed("GET", "/api/health");
        bad.host = None;
        assert_eq!(route(&bad, &config(), &token()).status, 403);
    }

    #[test]
    fn allowed_hosts_come_from_config() {
        let mut other = authed("GET", "/api/health");
        other.host = Some("alias.test:1".to_string());
        assert_eq!(route(&other, &config(), &token()).status, 200);
        let mut empty = config();
        empty.allowed_hosts.clear();
        assert_eq!(
            route(&authed("GET", "/api/health"), &empty, &token()).status,
            403
        );
    }

    #[test]
    fn overloaded_is_503_json_no_store() {
        let response = overloaded();
        assert_eq!(response.status, 503);
        assert_eq!(body(&response), json!({"error": "busy"}));
        assert!(response.headers.contains(&("Cache-Control", "no-store")));
    }

    #[test]
    fn every_response_is_json_no_store() {
        let mut bad_host = authed("GET", "/api/health");
        bad_host.host = None;
        let responses = [
            route(&authed("GET", "/api/health"), &config(), &token()),
            route(&request("GET", "/api/health", ""), &config(), &token()),
            route(&authed("GET", "/nope"), &config(), &token()),
            route(&authed("POST", "/api/health"), &config(), &token()),
            route(&bad_host, &config(), &token()),
        ];
        for response in responses {
            assert_eq!(response.content_type, "application/json");
            assert!(response.headers.contains(&("Cache-Control", "no-store")));
        }
    }
}
