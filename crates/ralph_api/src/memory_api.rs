use crate::memory::json::{entries_body, root_body, threads_body, zoom_body};
use crate::memory::query::{parse_query, MemoryQuery, QueryError, Scope};
use crate::route::{error, json_ok, ApiConfig, ApiRequest, ApiResponse};
use ralph_core::chat_memory::{page, root_view, zoom_view};
use ralph_server::chat::memory::{list_threads, peek, thread_exists, Peek, ThreadInfo};
use ralph_server::chat::thread::thread_dir;
use std::path::{Path, PathBuf};

const TOKEN_KEY: &str = "token";

fn not_found() -> ApiResponse {
    error(404, "not found")
}

fn unavailable() -> ApiResponse {
    error(500, "memory unavailable")
}

fn tree_response(query: &MemoryQuery, data: &Peek) -> ApiResponse {
    let Some(node) = query.node else {
        let nodes = root_view(data.entries.len(), &data.tree);
        return json_ok(root_body(&query.thread, data.entries.len(), &nodes));
    };
    match zoom_view(&data.entries, &data.tree, node) {
        Ok(view) => json_ok(zoom_body(&query.thread, &view)),
        Err(_) => not_found(),
    }
}

fn entries_response(query: &MemoryQuery, data: &Peek) -> ApiResponse {
    if query.node.is_some() {
        return error(400, QueryError::UnknownKey.message());
    }
    let chosen = page(&data.entries, query.after, query.limit);
    json_ok(entries_body(&query.thread, data.entries.len(), &chosen))
}

fn threads_response(threads: &[ThreadInfo]) -> ApiResponse {
    json_ok(threads_body(threads))
}

fn only_token_keys(raw: &str) -> bool {
    raw.split('&')
        .filter(|segment| !segment.is_empty())
        .all(|segment| segment.split_once('=').map_or(segment, |(key, _)| key) == TOKEN_KEY)
}

fn scope_dir(main: &Path, scope: &Scope) -> Option<PathBuf> {
    match scope {
        Scope::Main => Some(main.to_path_buf()),
        Scope::Thread(name) => thread_exists(main, name).then(|| thread_dir(main, name)),
    }
}

fn read_scope(
    request: &ApiRequest,
    config: &ApiConfig,
    render: fn(&MemoryQuery, &Peek) -> ApiResponse,
) -> ApiResponse {
    let query = match parse_query(&request.query) {
        Ok(query) => query,
        Err(failure) => return error(400, failure.message()),
    };
    let Some(dir) = scope_dir(&config.store, &query.thread) else {
        return not_found();
    };
    match peek(&dir) {
        Ok(data) => render(&query, &data),
        Err(_) => unavailable(),
    }
}

pub(crate) fn memory(request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    read_scope(request, config, tree_response)
}

pub(crate) fn entries(request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    read_scope(request, config, entries_response)
}

pub(crate) fn threads(request: &ApiRequest, config: &ApiConfig) -> ApiResponse {
    if !only_token_keys(&request.query) {
        return error(400, QueryError::UnknownKey.message());
    }
    match list_threads(&config.store) {
        Ok(found) => threads_response(&found),
        Err(_) => unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ralph_core::chat_memory::{Entry, Node, Role, Tree};
    use serde_json::Value;

    fn data(count: usize) -> Peek {
        let entries = (0..count)
            .map(|i| Entry {
                role: Role::User,
                at: format!("2026-03-01T09:{i:02}:00Z"),
                text: format!("entry {i}"),
                corrupt: false,
            })
            .collect();
        Peek {
            entries,
            tree: Tree::new(),
        }
    }

    fn query(node: Option<&str>) -> MemoryQuery {
        MemoryQuery {
            thread: Scope::Main,
            node: node.map(|text| Node::parse(text).unwrap()),
            after: 0,
            limit: 2,
        }
    }

    fn body(response: &ApiResponse) -> Value {
        serde_json::from_slice(&response.body).unwrap()
    }

    #[test]
    fn tree_without_node_is_root() {
        let response = tree_response(&query(None), &data(3));
        assert_eq!(response.status, 200);
        assert_eq!(body(&response)["entries"], 3);
    }

    #[test]
    fn tree_with_node_zooms() {
        let response = tree_response(&query(Some("0-1")), &data(3));
        assert_eq!(response.status, 200);
        assert_eq!(body(&response)["children"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn tree_node_beyond_total_is_404() {
        let response = tree_response(&query(Some("0-7")), &data(3));
        assert_eq!(response.status, 404);
    }

    #[test]
    fn entries_page_respects_limit() {
        let response = entries_response(&query(None), &data(3));
        assert_eq!(body(&response)["items"].as_array().unwrap().len(), 2);
        assert_eq!(body(&response)["next_after"], 2);
    }

    #[test]
    fn entries_reject_node() {
        let response = entries_response(&query(Some("0-1")), &data(3));
        assert_eq!(response.status, 400);
    }

    #[test]
    fn threads_response_lists_names() {
        let info = ThreadInfo {
            name: "side".to_string(),
            parked: false,
            merged: true,
        };
        let response = threads_response(&[info]);
        assert_eq!(body(&response)["threads"][0]["name"], "side");
    }

    #[test]
    fn only_token_keys_accepts_token_and_empty() {
        assert!(only_token_keys(""));
        assert!(only_token_keys("token=abc"));
        assert!(only_token_keys("token=a&&token"));
    }

    #[test]
    fn only_token_keys_rejects_other_keys() {
        assert!(!only_token_keys("thread=main"));
        assert!(!only_token_keys("token=a&limit=1"));
        assert!(!only_token_keys("tokens=a"));
    }
}
