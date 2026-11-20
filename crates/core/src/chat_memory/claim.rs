use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatLease {
    pub pid: u32,
    #[serde(default)]
    pub holder: Option<String>,
    #[serde(default)]
    pub generation: u64,
}

impl ChatLease {
    pub fn render(&self) -> String {
        json!(self).to_string()
    }
}

pub fn parse_lease(contents: &str) -> Option<ChatLease> {
    let text = contents.trim();
    match text.parse() {
        Ok(pid) => Some(ChatLease {
            pid,
            holder: None,
            generation: 0,
        }),
        Err(_) => serde_json::from_str(text).ok(),
    }
}

pub fn next_lease(previous: &str, pid: u32, holder: String) -> ChatLease {
    ChatLease {
        pid,
        holder: Some(holder),
        generation: parse_lease(previous)
            .map_or(0, |lease| lease.generation)
            .saturating_add(1),
    }
}

pub fn instance_id(pid: u32, start: &str) -> String {
    format!("{pid}@{start}")
}

pub fn holder_pid(contents: &str) -> Option<u32> {
    parse_lease(contents).map(|lease| lease.pid)
}

pub fn store_in_use_message(store: &Path, holder: Option<u32>) -> String {
    let by = holder
        .map(|pid| format!(" by pid {pid}"))
        .unwrap_or_default();
    format!(
        "store {} is in use{by}; use --store <DIR> or RALPH_CHAT_STORE for a second chat",
        store.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_LEASE: &str =
        r#"{"pid":4242,"holder":"4242@Fri Nov 20 13:16:08 2026","generation":3}"#;

    fn main_0475_holder_pid(contents: &str) -> Option<u32> {
        contents.trim().parse().ok()
    }

    fn lease(pid: u32, holder: Option<&str>, generation: u64) -> ChatLease {
        ChatLease {
            pid,
            holder: holder.map(str::to_string),
            generation,
        }
    }

    #[test]
    fn parses_a_pid_and_rejects_garbage() {
        assert_eq!(holder_pid("4242\n"), Some(4242));
        assert_eq!(holder_pid(""), None);
        assert_eq!(holder_pid("abc"), None);
    }

    #[test]
    fn new_reader_parses_the_old_pid_only_lease() {
        assert_eq!(parse_lease("4242"), Some(lease(4242, None, 0)));
        assert_eq!(parse_lease("4242\n"), Some(lease(4242, None, 0)));
        assert_eq!(parse_lease(""), None);
        assert_eq!(parse_lease("abc"), None);
        assert_eq!(parse_lease(r#"{"holder":"x"}"#), None);
    }

    #[test]
    fn new_reader_parses_the_new_lease() {
        let new = lease(4242, Some("4242@Fri Nov 20 13:16:08 2026"), 3);
        assert_eq!(new.render(), NEW_LEASE);
        assert_eq!(parse_lease(NEW_LEASE), Some(new));
        assert_eq!(holder_pid(NEW_LEASE), Some(4242));
    }

    #[test]
    fn old_reader_still_refuses_but_drops_the_pid_on_a_new_lease() {
        assert_eq!(main_0475_holder_pid("4242"), Some(4242));
        assert_eq!(main_0475_holder_pid(NEW_LEASE), None);
    }

    #[test]
    fn each_claim_bumps_the_generation_of_the_previous_lease() {
        assert_eq!(next_lease("", 7, "h".into()), lease(7, Some("h"), 1));
        assert_eq!(next_lease("4242", 7, "h".into()), lease(7, Some("h"), 1));
        assert_eq!(next_lease(NEW_LEASE, 7, "h".into()), lease(7, Some("h"), 4));
        assert_eq!(
            next_lease(&lease(1, None, u64::MAX).render(), 7, "h".into()).generation,
            u64::MAX
        );
    }

    #[test]
    fn instance_id_joins_the_pid_and_its_start_time() {
        assert_eq!(
            instance_id(4242, "Fri Nov 20 13:16:08 2026"),
            "4242@Fri Nov 20 13:16:08 2026"
        );
    }

    #[test]
    fn message_names_the_holder_when_known() {
        let store = Path::new("/s");
        assert_eq!(
            store_in_use_message(store, Some(7)),
            "store /s is in use by pid 7; use --store <DIR> or RALPH_CHAT_STORE for a second chat"
        );
        assert_eq!(
            store_in_use_message(store, None),
            "store /s is in use; use --store <DIR> or RALPH_CHAT_STORE for a second chat"
        );
    }
}
