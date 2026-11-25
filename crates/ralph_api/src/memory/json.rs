use super::query::Scope;
use super::stats::{node_stats, tree_stats, NodeStats};
use ralph_core::chat_memory::{Entry, EntryView, NodeView, Page, Tree, ZoomChild, ZoomView};
use ralph_server::chat::memory::ThreadInfo;
use serde_json::{json, Value};
use std::path::Path;

fn node_json(view: &NodeView, tree: &Tree) -> Value {
    let NodeStats {
        depth,
        children,
        blocks,
        summarised,
        pending,
    } = node_stats(&view.node, tree);
    json!({
        "id": view.node.id(),
        "lo": view.node.lo,
        "hi": view.node.hi,
        "leaf": view.is_leaf,
        "summary": view.summary,
        "stats": {
            "depth": depth,
            "children": children,
            "blocks": blocks,
            "summarised": summarised,
            "pending": pending,
        },
    })
}

fn entry_json(view: &EntryView) -> Value {
    json!({
        "i": view.i,
        "role": view.role,
        "at": view.at,
        "text": view.text,
    })
}

fn child_json(child: &ZoomChild, tree: &Tree) -> Value {
    match child {
        ZoomChild::Block(view) => {
            let mut value = node_json(view, tree);
            value["kind"] = json!("block");
            value
        }
        ZoomChild::Entry(view) => {
            let mut value = entry_json(view);
            value["kind"] = json!("entry");
            value
        }
    }
}

pub(crate) fn root_body(
    scope: &Scope,
    entries: &[Entry],
    nodes: &[NodeView],
    tree: &Tree,
) -> Value {
    let stats = tree_stats(entries, tree);
    json!({
        "thread": scope.label(),
        "entries": entries.len(),
        "nodes": nodes.iter().map(|view| node_json(view, tree)).collect::<Vec<_>>(),
        "stats": {
            "entries": stats.entries,
            "blocks": stats.blocks,
            "summarised": stats.summarised,
            "pending": stats.pending,
            "last_at": stats.last_at,
        },
    })
}

pub(crate) fn zoom_body(scope: &Scope, view: &ZoomView, tree: &Tree) -> Value {
    let node = NodeView {
        node: view.node,
        summary: view.summary.clone(),
        is_leaf: view.node.size() == 1,
    };
    json!({
        "thread": scope.label(),
        "node": node_json(&node, tree),
        "children": view.children.iter().map(|child| child_json(child, tree)).collect::<Vec<_>>(),
    })
}

pub(crate) fn entries_body(scope: &Scope, total: usize, page: &Page) -> Value {
    json!({
        "thread": scope.label(),
        "entries": total,
        "items": page.items.iter().map(entry_json).collect::<Vec<_>>(),
        "next_after": page.next_after,
    })
}

pub(crate) fn store_name(store: &Path) -> String {
    store.file_name().map_or_else(
        || "store".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

pub(crate) fn threads_body(store: &str, threads: &[ThreadInfo]) -> Value {
    json!({
        "store": store,
        "threads": threads
            .iter()
            .map(|thread| json!({
                "name": thread.name,
                "parked": thread.parked,
                "merged": thread.merged,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::golden::assert_golden_in;
    use ralph_core::chat_memory::{page, root_view, zoom_view, Entry, Node, Role, Tree};
    use ralph_core::chat_thread::ThreadName;

    fn entry(role: Role, minute: usize, text: &str) -> Entry {
        Entry {
            role,
            at: format!("2026-03-01T09:{minute:02}:00Z"),
            text: text.to_string(),
            corrupt: false,
        }
    }

    fn fixture_entries() -> Vec<Entry> {
        vec![
            entry(Role::User, 0, "Can you check why the nightly sync keeps timing out?"),
            entry(
                Role::Assistant,
                1,
                "The sync job opens one connection per shard.\n\nPool size is 4, so shards 5 to 8 wait.",
            ),
            entry(Role::User, 2, "Raise the pool to 8 and rerun it."),
            entry(Role::Worker, 3, "worker w1 finished: cargo test passed (212 tests)"),
            entry(Role::Assistant, 4, "Done. The run took 41s instead of 3m."),
            entry(Role::User, 5, "He said \"stop\" and wrote C:\\temp\\new\\file \\u0041 \\n."),
            entry(Role::User, 6, "<script>alert(1)</script>"),
            entry(Role::Assistant, 7, "</r-prose><r-error>"),
            entry(Role::User, 8, "nul:\u{0}:end and line sep:\u{2028}:end and bell \u{7}"),
            entry(Role::Assistant, 9, "\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{202e}evil\u{202c} \u{645}\u{631}\u{62d}\u{628}\u{627}"),
            entry(Role::User, 10, &"x".repeat(20 * 1024)),
        ]
    }

    fn fixture_tree() -> Tree {
        let node = |lo, hi| Node::new(lo, hi).unwrap();
        [
            (
                node(0, 8),
                "Debugged the nightly sync timeout; pool raised to 8.",
            ),
            (node(0, 4), "Sync timeout traced to a pool of 4."),
            (node(8, 10), "<script>alert(1)</script> \"quoted\" \\ back"),
        ]
        .into_iter()
        .map(|(n, s)| (n, s.to_string()))
        .collect()
    }

    fn pretty(value: &Value) -> String {
        serde_json::to_string_pretty(value).unwrap()
    }

    fn plan() -> Scope {
        Scope::Thread(ThreadName::parse("plan-b").unwrap())
    }

    #[test]
    fn root_golden() {
        let tree = fixture_tree();
        let body = root_body(
            &Scope::Main,
            &fixture_entries(),
            &root_view(11, &tree),
            &tree,
        );
        assert_golden_in("memory", "json", "root", &pretty(&body));
    }

    #[test]
    fn root_empty_chat() {
        let tree = Tree::new();
        let body = root_body(&Scope::Main, &[], &root_view(0, &tree), &tree);
        assert_eq!(
            body,
            serde_json::json!({
                "thread": "main",
                "entries": 0,
                "nodes": [],
                "stats": {
                    "entries": 0,
                    "blocks": 0,
                    "summarised": 0,
                    "pending": 0,
                    "last_at": null,
                },
            })
        );
    }

    #[test]
    fn root_body_carries_the_stat_line_fields() {
        let tree = fixture_tree();
        let body = root_body(
            &Scope::Main,
            &fixture_entries(),
            &root_view(11, &tree),
            &tree,
        );
        assert_eq!(
            body["stats"],
            serde_json::json!({
                "entries": 11,
                "blocks": 8,
                "summarised": 3,
                "pending": 5,
                "last_at": "2026-03-01T09:10:00Z",
            })
        );
        assert_eq!(
            body["nodes"][0]["stats"],
            serde_json::json!({
                "depth": 3,
                "children": 2,
                "blocks": 7,
                "summarised": 2,
                "pending": 5,
            })
        );
    }

    #[test]
    fn zoom_block_golden() {
        let (entries, tree) = (fixture_entries(), fixture_tree());
        let view = zoom_view(&entries, &tree, Node::new(0, 8).unwrap()).unwrap();
        assert_golden_in(
            "memory",
            "json",
            "zoom_block",
            &pretty(&zoom_body(&plan(), &view, &tree)),
        );
    }

    #[test]
    fn zoom_entries_golden() {
        let (entries, tree) = (fixture_entries(), fixture_tree());
        let view = zoom_view(&entries, &tree, Node::new(8, 10).unwrap()).unwrap();
        assert_golden_in(
            "memory",
            "json",
            "zoom_entries",
            &pretty(&zoom_body(&Scope::Main, &view, &tree)),
        );
    }

    #[test]
    fn entries_first_page_golden() {
        let entries = fixture_entries();
        let body = entries_body(&Scope::Main, entries.len(), &page(&entries, 5, 4));
        assert_golden_in("memory", "json", "entries_page", &pretty(&body));
    }

    #[test]
    fn entries_last_page_has_null_next() {
        let entries = fixture_entries();
        let body = entries_body(&Scope::Main, entries.len(), &page(&entries, 9, 50));
        assert_eq!(body["next_after"], Value::Null);
        assert_eq!(body["items"].as_array().unwrap().len(), 2);
        assert_eq!(body["items"][0]["role"], "assistant");
    }

    #[test]
    fn entry_text_is_untouched() {
        let entries = fixture_entries();
        let body = entries_body(&Scope::Main, entries.len(), &page(&entries, 0, 200));
        for (i, source) in entries.iter().enumerate() {
            assert_eq!(body["items"][i]["text"], source.text.as_str());
        }
        assert_eq!(body["items"][3]["role"], "worker");
    }

    #[test]
    fn bodies_carry_no_text_field_besides_entries() {
        let (entries, tree) = (fixture_entries(), fixture_tree());
        let root = root_body(&Scope::Main, &entries, &root_view(11, &tree), &tree);
        assert!(!pretty(&root).contains("\"text\""));
        let view = zoom_view(&entries, &tree, Node::new(0, 8).unwrap()).unwrap();
        assert!(!pretty(&zoom_body(&Scope::Main, &view, &tree)).contains("\"text\""));
    }

    #[test]
    fn threads_golden() {
        let threads = vec![
            ThreadInfo {
                name: "plan-b".to_string(),
                parked: false,
                merged: false,
            },
            ThreadInfo {
                name: "spike".to_string(),
                parked: true,
                merged: false,
            },
            ThreadInfo {
                name: "old".to_string(),
                parked: false,
                merged: true,
            },
        ];
        assert_golden_in(
            "memory",
            "json",
            "threads",
            &pretty(&threads_body("store", &threads)),
        );
    }

    #[test]
    fn threads_empty() {
        assert_eq!(
            threads_body("s", &[]),
            serde_json::json!({"store": "s", "threads": []})
        );
    }

    #[test]
    fn store_name_is_the_last_component_only() {
        assert_eq!(
            store_name(Path::new("/home/me/secret/chat-store")),
            "chat-store"
        );
        assert_eq!(
            store_name(Path::new("/home/me/secret/chat-store/")),
            "chat-store"
        );
        assert_eq!(store_name(Path::new("relative")), "relative");
    }

    #[test]
    fn store_name_falls_back_without_a_final_component() {
        assert_eq!(store_name(Path::new("/")), "store");
        assert_eq!(store_name(Path::new("")), "store");
        assert_eq!(store_name(Path::new("..")), "store");
    }
}
