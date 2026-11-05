use crate::chat::schedule::{ask_yes_no, read_answer};
use crate::chat::thread::{
    config_root, list, merge_precheck, merge_thread, park_thread, pull_thread, rebase_thread,
    running_ids, thread_dir,
};
use crate::cli::NAP_TIMEOUT_SECS;
use crate::{run_curator, Curator};
use ralph_core::chat_memory::CallSite;
use ralph_core::chat_thread_cmd::{merge_question, op_refusal, parked_message, ThreadOp};
use std::io;
use std::path::Path;

pub(super) fn run(
    op: &ThreadOp,
    store_dir: &Path,
    in_thread: bool,
    confirm: &mut impl FnMut(&str) -> bool,
) -> Result<String, String> {
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
            park_thread(&main, name, &running_ids(&main, name)).map(|()| parked_message(name))
        }
        ThreadOp::Merge { name, rebase: true } => {
            rebase_thread(&main, name, &running_ids(&main, name))
                .map(|message| format!("rebased thread {name} into main's inbox\n{message}"))
        }
        ThreadOp::Merge {
            name,
            rebase: false,
        } => {
            let running = running_ids(&main, name);
            merge_precheck(&main, name, &running)?;
            if !confirm(&merge_question(name)) {
                return Ok("merge cancelled.".to_string());
            }
            println!("merging {name} (one paid Haiku call)...");
            let dir = thread_dir(&main, name);
            let curator = Curator {
                kind: CallSite::Merge,
                store: &dir,
                run: None,
                timeout: NAP_TIMEOUT_SECS,
            };
            merge_thread(&main, name, &running, &mut |prompt: &str| {
                run_curator(prompt, &curator)
            })
            .map(|message| format!("merged thread {name} into main's inbox\n{message}"))
        }
    }
}

pub(super) fn confirm(question: &str) -> bool {
    ask_yes_no(|| question.to_string(), &mut read_answer, &mut io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::memory::Store;
    use crate::chat::thread::ensure;
    use ralph_core::chat_thread::ThreadName;

    #[test]
    fn the_merge_precheck_refuses_before_any_question() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let auth = ThreadName::parse("auth").unwrap();
        let unknown = merge_precheck(&main, &auth, &[]).unwrap_err();
        assert!(unknown.starts_with("no thread auth"), "{unknown}");
        ensure(&main, &auth).unwrap();
        assert_eq!(merge_precheck(&main, &auth, &[]), Ok(()));
        let busy = merge_precheck(&main, &auth, &["w1".to_string()]).unwrap_err();
        assert!(busy.contains("running workers (w1)"), "{busy}");
        let held = Store::new(thread_dir(&main, &auth))
            .claim_chat()
            .unwrap()
            .unwrap();
        let open = merge_precheck(&main, &auth, &[]).unwrap_err();
        assert!(open.contains("in use"), "{open}");
        drop(held);
        assert_eq!(merge_precheck(&main, &auth, &[]), Ok(()));
    }
}
