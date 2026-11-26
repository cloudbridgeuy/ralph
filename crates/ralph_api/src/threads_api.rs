use crate::memory::json::store_name;
use crate::memory_api::check_token_keys;
use crate::route::{error, json_ok, ApiConfig, ApiRequest, ApiResponse};
use crate::threads::json::threads_body;
use chrono::Utc;
use ralph_server::chat::memory::thread_rows;

pub(crate) fn threads(request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    if let Err(failure) = check_token_keys(&request.query) {
        return error(400, failure.message());
    }
    match thread_rows(&config.store) {
        Ok(snapshot) => json_ok(threads_body(
            &store_name(&config.store),
            &snapshot,
            Utc::now(),
        )),
        Err(_) => error(500, "threads unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::route;
    use crate::token::Token;
    use ralph_core::chat_memory::Role;
    use ralph_core::chat_thread::ThreadName;
    use ralph_core::chat_thread::{ThreadMeta, THREAD_FILE};
    use ralph_server::chat::memory::Store;
    use ralph_server::chat::thread::thread_dir;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};

    const TOKEN: &str = "t0k3n";

    fn config(store: &Path) -> ApiConfig {
        ApiConfig {
            store: store.to_path_buf(),
            allowed_hosts: vec!["h.test:1".to_string()],
            version: "9.9.9".to_string(),
            tokens_css: String::new(),
            cookie_name: "ralph_token_1".to_string(),
            global_config: PathBuf::from("/secret/global/config.toml"),
            env: |_| None,
        }
    }

    fn request(method: &str, query: &str) -> ApiRequest {
        ApiRequest {
            method: method.to_string(),
            path: "/api/threads".to_string(),
            query: query.to_string(),
            host: Some("h.test:1".to_string()),
            authorization: None,
            cookies: vec![],
            body: vec![],
            content_type: None,
            body_too_large: false,
        }
    }

    fn send(method: &str, query: &str, store: &Path) -> ApiResponse {
        route(
            &request(method, &format!("token={TOKEN}{query}")),
            &config(store),
            &Token::from_value(TOKEN),
        )
    }

    fn body(response: &ApiResponse) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    fn seeded_store() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let main = root.path();
        let store = Store::new(main.to_path_buf());
        for i in 0..3 {
            store.append(Role::User, &format!("note {i}")).unwrap();
        }
        let name = ThreadName::parse("lint-sweep").unwrap();
        let dir = thread_dir(main, &name);
        fs::create_dir_all(&dir).unwrap();
        let meta = ThreadMeta {
            base: 2,
            created: "2026-01-02T00:00:00Z".to_string(),
            merged: None,
            parked: None,
            auto: None,
            model: None,
            closed: false,
        };
        fs::write(dir.join(THREAD_FILE), serde_json::to_string(&meta).unwrap()).unwrap();
        root
    }

    #[test]
    fn threads_route_returns_rows_and_graph() {
        let root = seeded_store();
        let response = send("GET", "", root.path());
        assert_eq!(response.status, 200);
        assert!(response.headers.contains(&("Cache-Control", "no-store")));
        let json = body(&response);
        assert_eq!(json["head"]["messages"], 3);
        assert_eq!(json["threads"][0]["name"], "lint-sweep");
        assert_eq!(json["threads"][0]["state"], "open");
        assert_eq!(json["graph"][0]["kind"], "thread");
        assert!(json["graph"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["kind"] == "head"));
        let leaked = root.path().to_string_lossy().into_owned();
        assert!(!String::from_utf8(response.body).unwrap().contains(&leaked));
    }

    #[test]
    fn threads_route_rejects_query_and_post() {
        let root = seeded_store();
        for query in ["&thread=main", "&limit=1", "&x"] {
            let response = send("GET", query, root.path());
            assert_eq!(response.status, 400, "{query}");
            assert_eq!(body(&response)["error"], "unknown query key");
        }
        let response = send("POST", "", root.path());
        assert_eq!(response.status, 405);
        assert!(response.headers.contains(&("Allow", "GET")));
    }

    #[test]
    fn threads_route_500_has_no_path() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("not-a-store");
        fs::write(&file, "x").unwrap();
        let response = send("GET", "", &file);
        assert_eq!(response.status, 500);
        assert_eq!(
            body(&response),
            serde_json::json!({"error": "threads unavailable"})
        );
        let text = String::from_utf8(response.body).unwrap();
        assert!(!text.contains(&root.path().to_string_lossy().into_owned()));
    }
}
