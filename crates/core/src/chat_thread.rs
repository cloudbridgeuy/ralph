use crate::chat_memory::{Tool, ViewSplit};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const THREADS_DIR: &str = "threads";
pub const THREAD_FILE: &str = "thread.json";
pub const MAIN_PREFIX_LINES: usize = 100;
pub const MAX_NAME_CHARS: usize = 32;
const MAIN: &str = "main";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThreadName(String);

impl ThreadName {
    pub fn parse(s: &str) -> Result<ThreadName, String> {
        let valid = (1..=MAX_NAME_CHARS).contains(&s.len())
            && !s.starts_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            return Err(format!(
                "'{s}' is not a thread name. Use 1 to {MAX_NAME_CHARS} lowercase letters, digits or dashes, not starting with a dash."
            ));
        }
        if s == MAIN {
            return Err("'main' is the main chat, not a thread name.".to_string());
        }
        Ok(ThreadName(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThreadName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMeta {
    pub base: usize,
    pub created: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Own,
    Main,
    Thread(ThreadName),
}

pub fn split_scope(id: &str) -> Result<(Scope, &str), String> {
    let id = id.trim().trim_start_matches('#');
    match id.split_once(':') {
        None => Ok((Scope::Main, id)),
        Some((MAIN, rest)) => Ok((Scope::Main, rest)),
        Some((name, rest)) => Ok((Scope::Thread(ThreadName::parse(name)?), rest)),
    }
}

pub fn scope_arg(args: &Value) -> Result<Scope, String> {
    match args.get("thread") {
        None | Some(Value::Null) => Ok(Scope::Own),
        Some(Value::String(s)) if s.trim() == MAIN => Ok(Scope::Main),
        Some(Value::String(s)) => ThreadName::parse(s.trim()).map(Scope::Thread),
        Some(_) => Err("'thread' must be a string.".to_string()),
    }
}

pub fn label_item(item: &str, name: &ThreadName) -> String {
    for mark in ["#", "[#"] {
        if let Some(rest) = item.strip_prefix(mark) {
            return format!("{mark}{name}:{rest}");
        }
    }
    item.to_string()
}

pub fn label_lines(text: &str, name: &ThreadName) -> String {
    text.lines()
        .map(|line| label_item(line, name))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn fork_check(tool: &Tool, base: usize) -> Result<(), String> {
    let first_after = match tool {
        Tool::Zoom(node) if node.hi > base => node.lo.max(base),
        Tool::Read(request) if request.to >= base => request.from.max(base),
        _ => return Ok(()),
    };
    Err(format!(
        "#{first_after} is after this thread's fork point (#{base}); pull to see it."
    ))
}

pub fn thread_split(
    main_prefix: Vec<String>,
    base: usize,
    name: &ThreadName,
    own: ViewSplit,
) -> ViewSplit {
    let label = |lines: Vec<String>| -> Vec<String> {
        lines.iter().map(|line| label_item(line, name)).collect()
    };
    let mut older = vec![format!(
        "[main chat before this thread forked at #{base}; its ids have no prefix]"
    )];
    older.extend(main_prefix);
    older.push(format!("[thread {name}; its ids look like #{name}:0]"));
    older.extend(label(own.older));
    ViewSplit {
        older,
        middle: label(own.middle),
        tail: label(own.tail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_memory::{Node, RangeRequest};
    use serde_json::json;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    #[test]
    fn thread_names_are_short_lowercase_words() {
        assert_eq!(name("auth-fix2").as_str(), "auth-fix2");
        assert!(ThreadName::parse(&"a".repeat(32)).is_ok());
        for bad in [
            "",
            "-x",
            "Auth",
            "a b",
            "a/b",
            "a:b",
            "main",
            &"a".repeat(33),
        ] {
            assert!(ThreadName::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn thread_meta_round_trips_as_json() {
        let meta = ThreadMeta {
            base: 42,
            created: "2026-11-02T10:00:00Z".to_string(),
        };
        let text = serde_json::to_string(&meta).unwrap();
        assert_eq!(text, r#"{"base":42,"created":"2026-11-02T10:00:00Z"}"#);
        assert_eq!(serde_json::from_str::<ThreadMeta>(&text).unwrap(), meta);
    }

    #[test]
    fn ids_without_a_prefix_belong_to_main() {
        assert_eq!(split_scope("4-7").unwrap(), (Scope::Main, "4-7"));
        assert_eq!(split_scope("#5").unwrap(), (Scope::Main, "5"));
        assert_eq!(split_scope("main:5").unwrap(), (Scope::Main, "5"));
        assert_eq!(
            split_scope("#auth:0-3").unwrap(),
            (Scope::Thread(name("auth")), "0-3")
        );
        assert!(split_scope("Bad:1").is_err());
    }

    #[test]
    fn the_thread_argument_picks_the_search_space() {
        assert_eq!(scope_arg(&json!({})).unwrap(), Scope::Own);
        assert_eq!(scope_arg(&json!({"thread": "main"})).unwrap(), Scope::Main);
        assert_eq!(
            scope_arg(&json!({"thread": "auth"})).unwrap(),
            Scope::Thread(name("auth"))
        );
        assert!(scope_arg(&json!({"thread": 3})).is_err());
    }

    #[test]
    fn labels_go_on_the_leading_id_only() {
        let auth = name("auth");
        assert_eq!(
            label_item("#4-7 talk about #2", &auth),
            "#auth:4-7 talk about #2"
        );
        assert_eq!(
            label_item("[#3: unreadable log line, skipped]", &auth),
            "[#auth:3: unreadable log line, skipped]"
        );
        assert_eq!(label_item("(end of range)", &auth), "(end of range)");
        assert_eq!(
            label_lines(
                "#1 · user · x\n#0 · user · y\n(zoom <id> for the full text)",
                &auth
            ),
            "#auth:1 · user · x\n#auth:0 · user · y\n(zoom <id> for the full text)"
        );
    }

    #[test]
    fn ids_at_or_after_the_fork_point_ask_for_a_pull() {
        let read = |from, to| {
            Tool::Read(RangeRequest {
                from,
                to,
                mode: crate::chat_memory::Mode::Raw,
                budget: 10,
                cursor: None,
            })
        };
        assert!(fork_check(&Tool::Zoom(Node::new(4, 8).unwrap()), 8).is_ok());
        assert!(fork_check(&read(0, 7), 8).is_ok());
        assert!(fork_check(&Tool::Recall("x".to_string()), 0).is_ok());
        assert_eq!(
            fork_check(&Tool::Zoom(Node::new(8, 9).unwrap()), 8).unwrap_err(),
            "#8 is after this thread's fork point (#8); pull to see it."
        );
        let first_after = "#8 is after this thread's fork point (#8); pull to see it.";
        assert_eq!(
            fork_check(&Tool::Zoom(Node::new(0, 16).unwrap()), 8).unwrap_err(),
            first_after
        );
        assert_eq!(fork_check(&read(5, 9), 8).unwrap_err(), first_after);
        assert_eq!(fork_check(&read(0, 8), 8).unwrap_err(), first_after);
    }

    #[test]
    fn a_thread_view_is_the_main_prefix_then_its_own_labelled_lines() {
        let own = ViewSplit {
            older: vec!["#0-1 started".to_string()],
            middle: vec!["#2 user x hi".to_string()],
            tail: vec!["#3 assistant x yo".to_string()],
        };
        let split = thread_split(vec!["#0-63 old work".to_string()], 64, &name("auth"), own);
        assert_eq!(
            split.older,
            vec![
                "[main chat before this thread forked at #64; its ids have no prefix]",
                "#0-63 old work",
                "[thread auth; its ids look like #auth:0]",
                "#auth:0-1 started",
            ]
        );
        assert_eq!(split.middle, vec!["#auth:2 user x hi"]);
        assert_eq!(split.tail, vec!["#auth:3 assistant x yo"]);
    }
}
