use crate::chat::thread::{config_root, list, park_thread, pull_thread, running_ids};
use ralph_core::chat_thread_cmd::{op_refusal, ThreadOp};
use std::path::Path;

pub(super) fn run(op: &ThreadOp, store_dir: &Path, in_thread: bool) -> Result<String, String> {
    if let Some(refusal) = op_refusal(in_thread, op) {
        return Err(refusal);
    }
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
        ThreadOp::Pull(name) => pull_thread(&main, name),
        ThreadOp::Park(name) => {
            park_thread(&main, name, &running_ids(&main, name)).map(|()| format!("parked {name}"))
        }
    }
}
