use crate::memory::json::{entries_body, root_body, store_name, threads_body, zoom_body};
use crate::memory::query::{parse_query, segment_key, MemoryQuery, QueryError, Scope};
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
        return json_ok(root_body(&query.thread, &data.entries, &nodes, &data.tree));
    };
    match zoom_view(&data.entries, &data.tree, node) {
        Ok(view) => json_ok(zoom_body(
            &query.thread,
            &view,
            data.entries.len(),
            &data.tree,
        )),
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

fn threads_response(store: &Path, threads: &[ThreadInfo]) -> ApiResponse {
    json_ok(threads_body(&store_name(store), threads))
}

pub(crate) fn check_token_keys(raw: &str) -> Result<(), QueryError> {
    for segment in raw.split('&').filter(|segment| !segment.is_empty()) {
        if segment_key(segment)? != TOKEN_KEY {
            return Err(QueryError::UnknownKey);
        }
    }
    Ok(())
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
    if let Err(failure) = check_token_keys(&request.query) {
        return error(400, failure.message());
    }
    match list_threads(&config.store) {
        Ok(found) => threads_response(&config.store, &found),
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
    fn zoom_path_lists_ancestors_from_the_root() {
        let response = tree_response(&query(Some("0-1")), &data(45));
        assert_eq!(
            body(&response)["path"],
            serde_json::json!(["0-31", "0-15", "0-7", "0-3", "0-1"])
        );
    }

    #[test]
    fn zoom_path_of_a_root_node_is_just_itself() {
        let response = tree_response(&query(Some("0-31")), &data(45));
        assert_eq!(body(&response)["path"], serde_json::json!(["0-31"]));
    }

    #[test]
    fn unknown_node_is_still_404() {
        let response = tree_response(&query(Some("32-63")), &data(45));
        assert_eq!(response.status, 404);
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
        let response = threads_response(Path::new("/a/b/chat"), &[info]);
        assert_eq!(body(&response)["threads"][0]["name"], "side");
        assert_eq!(body(&response)["store"], "chat");
        assert!(!String::from_utf8(response.body).unwrap().contains("/a/b"));
    }

    #[test]
    fn token_keys_accept_token_and_empty() {
        assert_eq!(check_token_keys(""), Ok(()));
        assert_eq!(check_token_keys("token=abc"), Ok(()));
        assert_eq!(check_token_keys("token=a&&token"), Ok(()));
        assert_eq!(check_token_keys("%74oken=a&to%6Ben"), Ok(()));
    }

    #[test]
    fn token_keys_reject_other_keys() {
        for raw in ["thread=main", "token=a&limit=1", "tokens=a", "%61fter=1"] {
            assert_eq!(check_token_keys(raw), Err(QueryError::UnknownKey), "{raw}");
        }
    }

    #[test]
    fn token_keys_reject_bad_escapes() {
        assert_eq!(check_token_keys("%7oken=a"), Err(QueryError::Encoding));
    }
}
