#![allow(clippy::unwrap_used, clippy::expect_used, reason = "test code")]
#[allow(dead_code, reason = "each test crate uses a subset of the helpers")]
mod common;

use common::{assert_golden, Host, Reply, TestServer};
use ralph_core::chat_memory::{
    encode_entry, encode_record, page, parse_log, zoom_view, Entry, NewEntry, Node, Role, Tree,
    ZoomChild,
};
use ralph_core::chat_thread::{THREADS_DIR, THREAD_FILE};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

const HOSTILE: [&str; 6] = [
    "He said \"stop\" and wrote C:\\temp\\new\\file \\u0041 \\n.",
    "</script><img src=x onerror=alert(1)>",
    "line one\nline two\r\n\ttabbed",
    "emoji \u{1F600} and \u{2028} separator and \u{0001} control",
    "plain",
    "last",
];

fn new_entry(i: usize, text: &str) -> NewEntry {
    let role = [Role::User, Role::Assistant, Role::Worker][i % 3];
    NewEntry {
        role,
        at: format!("2026-03-01T09:{i:02}:00Z"),
        text: text.to_string(),
    }
}

fn write_store(dir: &Path, texts: &[&str], records: &[(Node, &str)]) {
    fs::create_dir_all(dir).unwrap();
    let log: String = texts
        .iter()
        .enumerate()
        .map(|(i, text)| encode_entry(&new_entry(i, text)).unwrap())
        .collect();
    fs::write(dir.join("log.jsonl"), log).unwrap();
    let tree: String = records
        .iter()
        .map(|(node, text)| encode_record(*node, text).unwrap())
        .collect();
    fs::write(dir.join("tree.jsonl"), tree).unwrap();
}

fn main_records() -> Vec<(Node, &'static str)> {
    vec![
        (Node { lo: 0, hi: 4 }, "first four: sync timeout"),
        (Node { lo: 0, hi: 2 }, "pair summary"),
        (Node { lo: 4, hi: 6 }, "tail pair \"quoted\""),
    ]
}

fn fixture(dir: &Path) {
    write_store(dir, &HOSTILE, &main_records());
}

fn fixture_with_thread(dir: &Path) {
    fixture(dir);
    let side = dir.join(THREADS_DIR).join("side");
    write_store(&side, &["side one", "side two", "side three"], &[]);
    fs::write(
        side.join(THREAD_FILE),
        json!({"base": 2, "created": "2026-03-02T00:00:00Z"}).to_string(),
    )
    .unwrap();
    let done = dir.join(THREADS_DIR).join("done");
    fs::create_dir_all(&done).unwrap();
    fs::write(
        done.join(THREAD_FILE),
        json!({
            "base": 1,
            "created": "2026-03-02T00:00:00Z",
            "parked": {"at": "2026-03-03T00:00:00Z"},
            "merged": {"at": "2026-03-04T00:00:00Z", "digest": "d"}
        })
        .to_string(),
    )
    .unwrap();
}

fn get(server: &TestServer, target: &str) -> Reply {
    let sep = if target.contains('?') { '&' } else { '?' };
    let full = format!("{target}{sep}token={}", server.token());
    server.exchange("GET", &full, Host::Own, &[])
}

fn json_of(reply: &Reply) -> Value {
    serde_json::from_slice(&reply.body_bytes).unwrap()
}

fn error_of(reply: &Reply, status: u16, message: &str) {
    assert_eq!(reply.status, status, "{}", reply.body);
    assert_eq!(json_of(reply), json!({ "error": message }));
}

fn fixture_entries() -> Vec<Entry> {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    parse_log(&fs::read(dir.path().join("log.jsonl")).unwrap()).0
}

fn entry_json(entry: &Entry, i: usize) -> Value {
    json!({
        "i": i,
        "role": serde_json::to_value(entry.role).unwrap(),
        "at": entry.at,
        "text": entry.text,
    })
}

#[test]
fn root_lists_blocks_with_summaries() {
    let server = TestServer::start_with_store(fixture);
    let reply = get(&server, "/api/memory");
    assert_eq!(reply.status, 200);
    assert_eq!(
        json_of(&reply),
        json!({
            "thread": "main",
            "entries": 6,
            "nodes": [
                {"id": "0-3", "lo": 0, "hi": 4, "leaf": false, "summary": "first four: sync timeout"},
                {"id": "4-5", "lo": 4, "hi": 6, "leaf": false, "summary": "tail pair \"quoted\""},
            ],
        })
    );
}

#[test]
fn success_headers_are_private() {
    let server = TestServer::start_with_store(fixture);
    for target in ["/api/memory", "/api/memory/entries", "/api/memory/threads"] {
        let reply = get(&server, target);
        assert_eq!(reply.status, 200);
        let header = |name: &str| {
            reply
                .headers
                .iter()
                .find(|(found, _)| found.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(header("Content-Type"), Some("application/json"));
        assert_eq!(header("Cache-Control"), Some("no-store"));
        assert_eq!(header("Referrer-Policy"), Some("no-referrer"));
        assert_eq!(header("X-Content-Type-Options"), Some("nosniff"));
    }
}

#[test]
fn zoom_block_matches_core_view() {
    let server = TestServer::start_with_store(fixture);
    let entries = fixture_entries();
    let tree: Tree = main_records()
        .into_iter()
        .map(|(node, text)| (node, text.to_string()))
        .collect();
    let node = Node { lo: 0, hi: 4 };
    let view = zoom_view(&entries, &tree, node).unwrap();
    let expected: Vec<Value> = view
        .children
        .iter()
        .map(|child| match child {
            ZoomChild::Block(block) => json!({
                "id": block.node.id(),
                "lo": block.node.lo,
                "hi": block.node.hi,
                "leaf": block.is_leaf,
                "summary": block.summary,
                "kind": "block",
            }),
            ZoomChild::Entry(item) => {
                let mut value = entry_json(&entries[item.i], item.i);
                value["kind"] = json!("entry");
                value
            }
        })
        .collect();
    let reply = get(&server, "/api/memory?node=0-3");
    assert_eq!(reply.status, 200);
    let body = json_of(&reply);
    assert_eq!(body["children"], Value::Array(expected));
    assert_eq!(body["node"]["summary"], "first four: sync timeout");
}

#[test]
fn zoom_leaf_pair_returns_byte_exact_entries() {
    let server = TestServer::start_with_store(fixture);
    let entries = fixture_entries();
    let reply = get(&server, "/api/memory?node=0-1");
    assert_eq!(reply.status, 200);
    let body = json_of(&reply);
    let mut first = entry_json(&entries[0], 0);
    first["kind"] = json!("entry");
    let mut second = entry_json(&entries[1], 1);
    second["kind"] = json!("entry");
    assert_eq!(body["children"], json!([first, second]));
    assert_eq!(body["children"][0]["text"], HOSTILE[0]);
    assert_eq!(body["children"][1]["text"], HOSTILE[1]);
}

#[test]
fn entries_first_and_last_page_match_core_page() {
    let server = TestServer::start_with_store(fixture);
    let entries = fixture_entries();
    let first = json_of(&get(&server, "/api/memory/entries?limit=4"));
    let core = page(&entries, 0, 4);
    assert_eq!(first["entries"], 6);
    assert_eq!(first["next_after"], json!(core.next_after));
    assert_eq!(first["next_after"], 4);
    let expected: Vec<Value> = core
        .items
        .iter()
        .map(|item| entry_json(&entries[item.i], item.i))
        .collect();
    assert_eq!(first["items"], Value::Array(expected));
    let last = json_of(&get(&server, "/api/memory/entries?limit=4&after=4"));
    assert_eq!(last["next_after"], Value::Null);
    assert_eq!(last["items"].as_array().unwrap().len(), 2);
    assert_eq!(last["items"][0]["text"], HOSTILE[4]);
}

#[test]
fn hostile_text_survives_json_exactly() {
    let server = TestServer::start_with_store(fixture);
    let body = json_of(&get(&server, "/api/memory/entries"));
    let texts: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, HOSTILE);
}

#[test]
fn threads_lists_sorted_flags() {
    let server = TestServer::start_with_store(fixture_with_thread);
    let reply = get(&server, "/api/memory/threads");
    assert_eq!(reply.status, 200);
    assert_eq!(
        json_of(&reply),
        json!({"store": store_name(&server), "threads": [
            {"name": "done", "parked": true, "merged": true},
            {"name": "side", "parked": false, "merged": false},
        ]})
    );
}

#[test]
fn threads_empty_without_threads_dir() {
    let server = TestServer::start_with_store(fixture);
    assert_eq!(
        json_of(&get(&server, "/api/memory/threads")),
        json!({"store": store_name(&server), "threads": []})
    );
}

fn store_name(server: &TestServer) -> String {
    server
        .store_path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn threads_name_the_store_folder_but_never_its_path() {
    let server = TestServer::start_with_store(fixture);
    let reply = get(&server, "/api/memory/threads");
    let text = reply.body;
    let path = server.store_path();
    assert!(text.contains(&store_name(&server)));
    assert!(!text.contains(path.to_str().unwrap()));
    assert!(!text.contains(path.parent().unwrap().to_str().unwrap()));
}

#[test]
fn named_thread_reads_its_own_log() {
    let server = TestServer::start_with_store(fixture_with_thread);
    let body = json_of(&get(&server, "/api/memory/entries?thread=side"));
    assert_eq!(body["thread"], "side");
    assert_eq!(body["entries"], 3);
    assert_eq!(body["items"][2]["text"], "side three");
    let root = json_of(&get(&server, "/api/memory?thread=side"));
    assert_eq!(root["thread"], "side");
    assert_eq!(root["entries"], 3);
}

#[test]
fn explicit_main_scope_equals_default() {
    let server = TestServer::start_with_store(fixture);
    assert_eq!(
        json_of(&get(&server, "/api/memory?thread=main")),
        json_of(&get(&server, "/api/memory"))
    );
}

#[test]
fn unknown_thread_is_404() {
    let server = TestServer::start_with_store(fixture_with_thread);
    error_of(&get(&server, "/api/memory?thread=ghost"), 404, "not found");
    error_of(
        &get(&server, "/api/memory/entries?thread=ghost"),
        404,
        "not found",
    );
}

#[test]
fn invalid_thread_name_is_400() {
    let server = TestServer::start_with_store(fixture);
    for target in [
        "/api/memory?thread=..%2Fx",
        "/api/memory?thread=a%20b",
        "/api/memory?thread=",
    ] {
        let reply = get(&server, target);
        assert_eq!(reply.status, 400, "{target}: {}", reply.body);
    }
}

#[test]
fn bad_node_is_400() {
    let server = TestServer::start_with_store(fixture);
    for target in [
        "/api/memory?node=abc",
        "/api/memory?node=3-1",
        "/api/memory?node=",
    ] {
        let reply = get(&server, target);
        assert_eq!(reply.status, 400, "{target}: {}", reply.body);
    }
}

#[test]
fn node_beyond_total_is_404() {
    let server = TestServer::start_with_store(fixture);
    error_of(&get(&server, "/api/memory?node=0-7"), 404, "not found");
    error_of(&get(&server, "/api/memory?node=6"), 404, "not found");
}

#[test]
fn unknown_and_duplicate_keys_are_400() {
    let server = TestServer::start_with_store(fixture);
    for target in [
        "/api/memory?bogus=1",
        "/api/memory?thread=main&thread=main",
        "/api/memory/entries?bogus=1",
        "/api/memory/entries?limit=1&limit=2",
    ] {
        let reply = get(&server, target);
        assert_eq!(reply.status, 400, "{target}: {}", reply.body);
    }
}

#[test]
fn entries_reject_node_and_threads_reject_any_key() {
    let server = TestServer::start_with_store(fixture);
    assert_eq!(get(&server, "/api/memory/entries?node=0-1").status, 400);
    for target in [
        "/api/memory/threads?thread=main",
        "/api/memory/threads?limit=1",
        "/api/memory/threads?x",
    ] {
        let reply = get(&server, target);
        assert_eq!(reply.status, 400, "{target}: {}", reply.body);
    }
}

#[test]
fn memory_ignores_after_and_limit() {
    let server = TestServer::start_with_store(fixture);
    assert_eq!(
        json_of(&get(&server, "/api/memory?after=3&limit=1")),
        json_of(&get(&server, "/api/memory"))
    );
}

#[test]
fn limit_is_clamped_and_bounded() {
    let texts: Vec<String> = (0..205).map(|i| format!("e{i}")).collect();
    let server = TestServer::start_with_store(|dir| {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        write_store(dir, &refs, &[]);
    });
    let big = json_of(&get(&server, "/api/memory/entries?limit=100000"));
    assert_eq!(big["items"].as_array().unwrap().len(), 200);
    assert_eq!(big["next_after"], 200);
    let default = json_of(&get(&server, "/api/memory/entries"));
    assert_eq!(default["items"].as_array().unwrap().len(), 50);
    assert_eq!(get(&server, "/api/memory/entries?limit=0").status, 400);
    assert_eq!(get(&server, "/api/memory/entries?limit=-1").status, 400);
    assert_eq!(get(&server, "/api/memory/entries?after=x").status, 400);
}

#[test]
fn missing_token_is_401() {
    let server = TestServer::start_with_store(fixture);
    for target in ["/api/memory", "/api/memory/entries", "/api/memory/threads"] {
        let reply = server.exchange("GET", target, Host::Own, &[]);
        error_of(&reply, 401, "unauthorized");
    }
}

#[test]
fn wrong_host_is_403() {
    let server = TestServer::start_with_store(fixture);
    let target = format!("/api/memory?token={}", server.token());
    let reply = server.exchange("GET", &target, Host::Named("evil.test"), &[]);
    error_of(&reply, 403, "forbidden host");
}

#[test]
fn bearer_and_query_tokens_both_work() {
    let server = TestServer::start_with_store(fixture);
    let bearer = format!("Bearer {}", server.token());
    let reply = server.exchange(
        "GET",
        "/api/memory",
        Host::Own,
        &[("Authorization", &bearer)],
    );
    assert_eq!(reply.status, 200);
    assert_eq!(get(&server, "/api/memory").status, 200);
}

#[test]
fn non_get_is_405_with_allow() {
    let server = TestServer::start_with_store(fixture);
    for target in ["/api/memory", "/api/memory/entries", "/api/memory/threads"] {
        let full = format!("{target}?token={}", server.token());
        for method in ["POST", "DELETE", "PUT"] {
            let reply = server.exchange(method, &full, Host::Own, &[]);
            assert_eq!(reply.status, 405, "{method} {target}");
            assert!(reply
                .headers
                .iter()
                .any(|(name, value)| name == "Allow" && value == "GET"));
        }
    }
}

#[test]
fn missing_store_is_500_without_path() {
    let server = TestServer::start();
    let store = server.store_path().to_path_buf();
    fs::remove_dir_all(&store).unwrap();
    for target in ["/api/memory", "/api/memory/entries"] {
        let reply = get(&server, target);
        error_of(&reply, 500, "memory unavailable");
        assert!(!reply.body.contains(store.to_str().unwrap()));
    }
}

#[test]
fn torn_tail_is_ignored() {
    let server = TestServer::start_with_store(|dir| {
        write_store(dir, &["one", "two", "three"], &[]);
        let mut log = fs::read(dir.join("log.jsonl")).unwrap();
        log.extend_from_slice(
            b"{\"role\":\"user\",\"at\":\"2026-03-01T10:00:00Z\",\"text\":\"half writ",
        );
        fs::write(dir.join("log.jsonl"), log).unwrap();
    });
    let body = json_of(&get(&server, "/api/memory/entries"));
    assert_eq!(body["entries"], 3);
    assert_eq!(body["items"].as_array().unwrap().len(), 3);
    assert!(!body.to_string().contains("half writ"));
}

#[test]
fn tree_node_past_log_is_hidden() {
    let server = TestServer::start_with_store(|dir| {
        write_store(
            dir,
            &["a", "b", "c"],
            &[
                (Node { lo: 0, hi: 2 }, "pair ok"),
                (Node { lo: 0, hi: 4 }, "ghost block"),
            ],
        );
    });
    let root = get(&server, "/api/memory");
    assert!(root.body.contains("pair ok"));
    assert!(!root.body.contains("ghost block"));
    error_of(&get(&server, "/api/memory?node=0-3"), 404, "not found");
}

type Snapshot = BTreeMap<String, (bool, Vec<u8>, Option<SystemTime>)>;

fn snapshot(root: &Path) -> Snapshot {
    let mut out = Snapshot::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let meta = fs::metadata(&dir).unwrap();
        let rel = dir.strip_prefix(root).unwrap().display().to_string();
        out.insert(rel, (true, Vec::new(), meta.modified().ok()));
        for item in fs::read_dir(&dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                let meta = fs::metadata(&path).unwrap();
                out.insert(rel, (false, fs::read(&path).unwrap(), meta.modified().ok()));
            }
        }
    }
    out
}

#[test]
fn reads_write_nothing() {
    let server = TestServer::start_with_store(fixture_with_thread);
    let before = snapshot(server.store_path());
    for target in [
        "/api/memory",
        "/api/memory?node=0-3",
        "/api/memory?thread=side",
        "/api/memory/entries",
        "/api/memory/entries?thread=side&limit=1",
        "/api/memory/threads",
        "/api/memory?thread=ghost",
    ] {
        get(&server, target);
    }
    let after = snapshot(server.store_path());
    assert_eq!(before, after);
    for name in ["LOCK", "log.idx", "tree.idx", "analytics", "history.jsonl"] {
        assert!(
            !after.keys().any(|path| path.ends_with(name)),
            "{name} appeared"
        );
    }
}

fn golden(name: &str, target: &str) {
    let server = TestServer::start_with_store(fixture_with_thread);
    let mut reply = get(&server, target);
    reply.body = reply.body.replace(&store_name(&server), "chat-store");
    assert_golden(name, &reply);
}

#[test]
fn golden_memory_root() {
    golden("memory_root", "/api/memory");
}

#[test]
fn golden_memory_entries_hostile() {
    golden("memory_entries", "/api/memory/entries");
}

#[test]
fn golden_memory_threads() {
    golden("memory_threads", "/api/memory/threads");
}

#[test]
fn threads_decode_query_keys_like_the_memory_routes() {
    let server = TestServer::start_with_store(fixture);
    let bearer = format!("Bearer {}", server.token());
    let bearer_get = |query: &str| {
        let target = format!("/api/memory/threads{query}");
        server.exchange("GET", &target, Host::Own, &[("Authorization", &bearer)])
    };
    let plain = bearer_get("?token=x");
    assert_eq!(plain.status, 200, "{}", plain.body);
    let encoded = bearer_get("?%74oken=x");
    assert_eq!(encoded.status, 200, "{}", encoded.body);
    assert_eq!(json_of(&encoded), json_of(&plain));
    error_of(&bearer_get("?%61fter=1"), 400, "unknown query key");
    error_of(
        &bearer_get("?%7oken=x"),
        400,
        "query is not valid percent-encoded UTF-8",
    );
}

#[test]
fn thread_with_corrupt_metadata_is_404() {
    let server = TestServer::start_with_store(|dir| {
        fixture_with_thread(dir);
        let broken = dir.join(THREADS_DIR).join("broken");
        write_store(&broken, &["one"], &[]);
        fs::write(broken.join(THREAD_FILE), "{nope").unwrap();
    });
    for target in [
        "/api/memory?thread=broken",
        "/api/memory/entries?thread=broken",
    ] {
        error_of(&get(&server, target), 404, "not found");
    }
    let listed = json_of(&get(&server, "/api/memory/threads"));
    assert!(!listed.to_string().contains("broken"));
}
