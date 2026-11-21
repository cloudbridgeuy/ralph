use ralph_core::chat_process::SessionKey;
use ralph_core::worker::{BlockedContent, BLOCKED_DIR};
use ralph_server::chat::serve_proto::BlockedMap;
use std::path::PathBuf;

pub(super) fn read_blocked(stores: &[(SessionKey, PathBuf)]) -> BlockedMap {
    let mut blocked = BlockedMap::new();
    for (key, store) in stores {
        let thread = match key {
            SessionKey::Main => None,
            SessionKey::Thread(name) => Some(name.to_string()),
        };
        let Ok(entries) = std::fs::read_dir(store.join(BLOCKED_DIR)) else {
            continue;
        };
        for entry in entries.flatten() {
            let parsed = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|marker| serde_json::from_str::<BlockedContent>(&marker).ok());
            if let Some(content) = parsed {
                let worker = entry.file_name().to_string_lossy().into_owned();
                blocked.insert((thread.clone(), worker), content);
            }
        }
    }
    blocked
}
