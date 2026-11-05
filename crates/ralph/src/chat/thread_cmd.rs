use crate::chat::thread::{config_root, list, running_ids};
use ralph_core::chat_thread_cmd::ThreadOp;
use std::path::Path;

pub(super) fn run(op: &ThreadOp, store_dir: &Path) -> Result<String, String> {
    let main = config_root(store_dir);
    match op {
        ThreadOp::Ls => {
            let rows = list(&main, &|name| running_ids(&main, name).len())
                .map_err(|e| format!("could not list threads: {e}"))?;
            Ok(if rows.is_empty() {
                "no threads".to_string()
            } else {
                rows
            })
        }
    }
}
