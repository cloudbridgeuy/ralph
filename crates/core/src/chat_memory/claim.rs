use std::path::Path;

pub fn holder_pid(contents: &str) -> Option<u32> {
    contents.trim().parse().ok()
}

pub fn store_in_use_message(store: &Path, holder: Option<u32>) -> String {
    let by = holder
        .map(|pid| format!(" by pid {pid}"))
        .unwrap_or_default();
    format!(
        "ralph chat: store {} is in use{by}; use --store <DIR> or RALPH_CHAT_STORE for a second chat",
        store.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_pid_and_rejects_garbage() {
        assert_eq!(holder_pid("4242\n"), Some(4242));
        assert_eq!(holder_pid(""), None);
        assert_eq!(holder_pid("abc"), None);
    }

    #[test]
    fn message_names_the_holder_when_known() {
        let store = Path::new("/s");
        assert_eq!(
            store_in_use_message(store, Some(7)),
            "ralph chat: store /s is in use by pid 7; use --store <DIR> or RALPH_CHAT_STORE for a second chat"
        );
        assert_eq!(
            store_in_use_message(store, None),
            "ralph chat: store /s is in use; use --store <DIR> or RALPH_CHAT_STORE for a second chat"
        );
    }
}
