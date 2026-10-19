use crate::chat::memory::Store;
use ralph_core::worker::HeadlessState;
use ralph_core::worker_check::{logged_reports, CheckMemory};
use std::collections::HashMap;

#[derive(Default)]
pub struct CheckState {
    pub memory: CheckMemory,
    pub panes: HashMap<String, String>,
    pub nudges: HashMap<String, Option<String>>,
    pub reports: HashMap<String, String>,
    pub heartbeats: HashMap<String, HeadlessState>,
}

impl CheckState {
    pub fn seeded(store: &Store) -> Self {
        Self {
            reports: store
                .load()
                .map(|(entries, _)| logged_reports(&entries))
                .unwrap_or_default(),
            ..Self::default()
        }
    }
}
