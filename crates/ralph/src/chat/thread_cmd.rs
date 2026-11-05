use crate::chat::schedule::{ask_answer, read_answer};
use crate::chat::thread::{
    config_root, list, merge_precheck, merge_thread, park_thread, pull_thread, thread_dir,
    view_thread,
};
use crate::cli::NAP_TIMEOUT_SECS;
use crate::{run_curator, Curator};
use ralph_core::chat_memory::CallSite;
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_cmd::{
    merge_answer, merge_question, merging_notice, op_refusal, parked_message, view_posted_message,
    ThreadOp,
};
use std::io;
use std::path::Path;

pub(super) fn run(
    op: &ThreadOp,
    store_dir: &Path,
    in_thread: bool,
    confirm: &mut impl FnMut(&str) -> Option<bool>,
    running_of: &impl Fn(&ThreadName) -> Vec<String>,
) -> Result<String, String> {
    if let Some(refusal) = op_refusal(in_thread, op) {
        return Err(refusal);
    }
    let main = config_root(store_dir);
    match op {
        ThreadOp::Ls => {
            let rows = list(&main, &|name| running_of(name).len())
                .map_err(|e| format!("could not list threads: {e}"))?;
            Ok(if rows.is_empty() {
                "no threads".to_string()
            } else {
                rows
            })
        }
        ThreadOp::Pull(name) => pull_thread(&main, name),
        ThreadOp::Park(name) => {
            park_thread(&main, name, &running_of(name)).map(|()| parked_message(name))
        }
        ThreadOp::Merge { name, view: true } => view_thread(&main, name, &running_of(name))
            .map(|message| view_posted_message(name, &message)),
        ThreadOp::Merge { name, view: false } => {
            let running = running_of(name);
            merge_precheck(&main, name, &running)?;
            if !merge_answer(confirm(&merge_question(name)))? {
                return Ok("merge cancelled.".to_string());
            }
            let running = running_of(name);
            println!("{}", merging_notice(name));
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

pub(super) fn confirm(question: &str) -> Option<bool> {
    ask_answer(|| question.to_string(), &mut read_answer, &mut io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::memory::Store;
    use crate::chat::thread::ensure;

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

    fn idle(_: &ThreadName) -> Vec<String> {
        Vec::new()
    }

    fn merge_auth() -> ThreadOp {
        ThreadOp::Merge {
            name: ThreadName::parse("auth").unwrap(),
            view: false,
        }
    }

    #[test]
    fn a_declined_merge_cancels_after_one_question() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let mut asked = 0;
        let mut confirm = |_: &str| {
            asked += 1;
            Some(false)
        };
        let result = run(&merge_auth(), &main, false, &mut confirm, &idle);
        assert_eq!(result, Ok("merge cancelled.".to_string()));
        assert_eq!(asked, 1);
    }

    #[test]
    fn an_aborted_merge_question_is_refused_not_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let refusal = run(&merge_auth(), &main, false, &mut |_: &str| None, &idle).unwrap_err();
        assert!(refusal.starts_with("merge aborted"), "{refusal}");
    }

    #[test]
    fn merging_an_unknown_thread_refuses_without_asking() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let refusal = run(&merge_auth(), &main, false, &mut confirm, &idle).unwrap_err();
        assert!(refusal.starts_with("no thread auth"), "{refusal}");
    }

    #[test]
    fn merging_from_a_thread_refuses_without_asking() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let refusal = run(&merge_auth(), &main, true, &mut confirm, &idle).unwrap_err();
        assert!(
            refusal.contains("Run /main, then /merge <name>."),
            "{refusal}"
        );
    }

    #[test]
    fn a_worker_that_starts_during_the_question_refuses_the_merge() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let lookups = std::cell::Cell::new(0);
        let running_of = |_: &ThreadName| {
            lookups.set(lookups.get() + 1);
            if lookups.get() == 1 {
                Vec::new()
            } else {
                vec!["w1".to_string()]
            }
        };
        let refusal = run(
            &merge_auth(),
            &main,
            false,
            &mut |_: &str| Some(true),
            &running_of,
        )
        .unwrap_err();
        assert!(refusal.contains("running workers (w1)"), "{refusal}");
        assert_eq!(lookups.get(), 2);
    }
}
