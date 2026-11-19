use super::{
    drop_precheck, drop_thread, merge_precheck, merge_thread, read_meta, running_ids, thread_dir,
    write_meta,
};
use crate::chat::schedule::read_answer;
use crate::chat::thread_cmd::{merge_line, record_cli_merge};
use crate::cli::MERGE_TIMEOUT_SECS;
use crate::{run_curator, Curator};
use ralph_core::chat_memory::CallSite;
use ralph_core::chat_thread::{AutoMode, ThreadName};
use ralph_core::chat_thread_auto::{
    clear_closed, close_answer, close_question, close_refusal, PendingClose,
};
use ralph_core::chat_thread_cmd::{merge_posted_message, merging_notice};
use ralph_server::chat::settle::pending;
use ralph_server::warn::warn_if_err;
use ralph_server::worker_mcp::Inbox;
use std::io::Write;
use std::path::Path;

pub(in crate::chat) fn scan(main: &Path, inbox: &Inbox) -> bool {
    warn_if_err(
        inbox.drain_closes(),
        "Failed to clear thread-closed reports",
    );
    let messages = settle(
        main,
        &mut |question: &str| {
            print!("{question} ");
            std::io::stdout().flush().ok();
            let line = read_answer();
            println!();
            line
        },
        &mut |name: &ThreadName, prompt: &str| {
            let dir = thread_dir(main, name);
            let curator = Curator {
                kind: CallSite::Merge,
                store: &dir,
                run: None,
                timeout: MERGE_TIMEOUT_SECS,
            };
            run_curator(prompt, &curator)
        },
        &|name: &ThreadName| running_ids(main, name),
    );
    messages.iter().for_each(|message| println!("{message}"));
    !messages.is_empty()
}

pub(super) fn settle(
    main: &Path,
    ask: &mut impl FnMut(&str) -> Option<String>,
    curate: &mut impl FnMut(&ThreadName, &str) -> Result<String, String>,
    running_of: &impl Fn(&ThreadName) -> Vec<String>,
) -> Vec<String> {
    pending(main)
        .into_iter()
        .map(|close| settle_one(main, &close, ask, curate, running_of))
        .collect()
}

fn settle_one(
    main: &Path,
    close: &PendingClose,
    ask: &mut impl FnMut(&str) -> Option<String>,
    curate: &mut impl FnMut(&ThreadName, &str) -> Result<String, String>,
    running_of: &impl Fn(&ThreadName) -> Vec<String>,
) -> String {
    let name = &close.name;
    let answer = ask(&close_question(name, close.auto));
    if !close_answer(answer.as_deref()) {
        clear_mark(main, name);
        return format!("thread {name} is kept.");
    }
    let done = match close.auto {
        AutoMode::Merge => merge(main, name, curate, running_of),
        AutoMode::Drop => drop_precheck(main, name, &running_of(name))
            .and_then(|()| drop_thread(main, name, &running_of(name))),
    };
    done.unwrap_or_else(|reason| {
        clear_mark(main, name);
        close_refusal(name, close.auto, &reason)
    })
}

fn merge(
    main: &Path,
    name: &ThreadName,
    curate: &mut impl FnMut(&ThreadName, &str) -> Result<String, String>,
    running_of: &impl Fn(&ThreadName) -> Vec<String>,
) -> Result<String, String> {
    merge_precheck(main, name, &running_of(name))?;
    println!("{}", merging_notice(name));
    let message = merge_thread(main, name, &running_of(name), &mut |prompt: &str| {
        curate(name, prompt)
    })?;
    let output = merge_posted_message(name, &message);
    record_cli_merge(main, name, &merge_line(name, false), &output);
    Ok(output)
}

fn clear_mark(main: &Path, name: &ThreadName) {
    let dir = thread_dir(main, name);
    let cleared = read_meta(&dir).ok().and_then(|meta| clear_closed(&meta));
    if let Some(meta) = cleared {
        warn_if_err(write_meta(&dir, &meta), "Failed to clear the closed mark");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::send_it::tests::held::monitor;
    use crate::chat::thread::tests::{forked, name};
    use crate::chat::thread::{read_meta, thread_dir, write_meta};
    use ralph_core::chat_thread::{AutoMode, ThreadMeta};
    use ralph_core::chat_thread_auto::{thread_closed_report, PendingClose};
    use ralph_core::inbox::Selection;
    use ralph_server::chat::memory::Store;
    use std::path::PathBuf;

    fn closed_auth(auto: AutoMode) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (root, main, dir) = forked();
        let meta = ThreadMeta {
            auto: Some(auto),
            closed: true,
            ..read_meta(&dir).unwrap()
        };
        write_meta(&dir, &meta).unwrap();
        (root, main, dir)
    }

    struct Run {
        asked: Vec<String>,
        curated: usize,
        messages: Vec<String>,
    }

    fn run(main: &Path, answer: Option<&str>, running: &[&str]) -> Run {
        let mut asked = Vec::new();
        let mut curated = 0;
        let running: Vec<String> = running.iter().map(|id| id.to_string()).collect();
        let messages = settle(
            main,
            &mut |question: &str| {
                asked.push(question.to_string());
                answer.map(str::to_string)
            },
            &mut |thread: &ThreadName, _: &str| {
                curated += 1;
                Ok(format!("done #{thread}:0"))
            },
            &|_: &ThreadName| running.clone(),
        );
        Run {
            asked,
            curated,
            messages,
        }
    }

    #[test]
    fn yes_merges_with_one_curator_call_and_one_question() {
        let (_root, main, dir) = closed_auth(AutoMode::Merge);
        let log_before = Store::new(main.clone()).log_len().unwrap();
        let ran = run(&main, Some("y"), &[]);
        assert_eq!(ran.asked, ["merge auth? [y/n]"]);
        assert_eq!(ran.curated, 1);
        let merged = read_meta(&dir).unwrap().merged.unwrap();
        assert_eq!(merged.digest, "done #auth:0");
        assert_eq!(Store::new(main.clone()).log_len().unwrap(), log_before + 1);
        assert!(ran.messages.iter().any(|m| m.contains("done #auth:0")));
        let again = run(&main, Some("y"), &[]);
        assert!(again.asked.is_empty());
        assert_eq!(again.curated, 0);
    }

    #[test]
    fn yes_drops_the_thread_with_one_question() {
        let (_root, main, dir) = closed_auth(AutoMode::Drop);
        let ran = run(&main, Some("yes"), &[]);
        assert_eq!(ran.asked, ["drop auth? [y/n]"]);
        assert_eq!(ran.curated, 0);
        assert!(!dir.exists());
        assert!(run(&main, Some("y"), &[]).asked.is_empty());
    }

    #[test]
    fn anything_but_yes_keeps_the_thread_and_clears_the_mark() {
        for answer in [Some("n"), None, Some("maybe"), Some("")] {
            let (_root, main, dir) = closed_auth(AutoMode::Merge);
            let log = Store::new(dir.clone()).log_len().unwrap();
            let ran = run(&main, answer, &[]);
            assert_eq!(ran.asked.len(), 1);
            assert_eq!(ran.curated, 0);
            let meta = read_meta(&dir).unwrap();
            assert_eq!((meta.closed, meta.auto), (false, Some(AutoMode::Merge)));
            assert_eq!(meta.merged, None);
            assert_eq!(Store::new(dir).log_len().unwrap(), log);
            assert!(run(&main, Some("y"), &[]).asked.is_empty());
        }
    }

    #[test]
    fn a_refused_merge_keeps_the_thread_and_names_the_manual_command() {
        let (_root, main, dir) = closed_auth(AutoMode::Merge);
        let held = Store::new(dir.clone()).claim_chat().unwrap().unwrap();
        let ran = run(&main, Some("y"), &[]);
        drop(held);
        assert_eq!(ran.asked.len(), 1);
        assert_eq!(ran.curated, 0);
        let said = ran.messages.join("\n");
        assert!(said.contains("in use"), "{said}");
        assert!(said.contains("ralph thread merge auth"), "{said}");
        let meta = read_meta(&dir).unwrap();
        assert_eq!((meta.closed, meta.merged), (false, None));
    }

    #[test]
    fn a_refused_drop_keeps_the_thread_and_names_the_manual_command() {
        let (_root, main, dir) = closed_auth(AutoMode::Drop);
        let ran = run(&main, Some("y"), &["w1"]);
        let said = ran.messages.join("\n");
        assert!(said.contains("running workers (w1)"), "{said}");
        assert!(said.contains("ralph thread drop auth"), "{said}");
        assert!(dir.exists());
        assert!(!read_meta(&dir).unwrap().closed);
    }

    #[test]
    fn nothing_pending_asks_nothing() {
        let (_root, main, dir) = forked();
        let ran = run(&main, Some("y"), &[]);
        assert!(ran.asked.is_empty() && ran.messages.is_empty());
        assert_eq!(ran.curated, 0);
        assert!(dir.exists());
        let empty = tempfile::tempdir().unwrap();
        assert!(run(empty.path(), Some("y"), &[]).asked.is_empty());
        assert!(thread_dir(&main, &name("auth")).exists());
    }

    #[test]
    fn a_main_store_with_a_closed_auto_thread_has_that_pending_close() {
        let (_root, main, _dir) = closed_auth(AutoMode::Drop);
        assert_eq!(
            pending(&main),
            [PendingClose {
                name: name("auth"),
                auto: AutoMode::Drop
            }]
        );
        let (_root, main, _dir) = forked();
        assert!(pending(&main).is_empty());
    }

    #[test]
    fn a_thread_chat_store_has_no_pending_close() {
        let (_root, _main, dir) = closed_auth(AutoMode::Merge);
        assert!(pending(&dir).is_empty());
    }

    #[test]
    fn refresh_clears_closes_after_settle() {
        let (_root, main, _dir) = closed_auth(AutoMode::Drop);
        let monitor = monitor(&main);
        monitor.refresh(false);
        assert_eq!(monitor.status().closes.len(), 1);
        run(&main, Some("y"), &[]);
        monitor.refresh(false);
        assert!(monitor.status().closes.is_empty());
    }

    #[test]
    fn a_close_report_is_never_drained_into_a_turn() {
        let dir = tempfile::tempdir().unwrap();
        let monitor = monitor(dir.path());
        let closed = thread_closed_report(&name("auth"), AutoMode::Merge, "t");
        monitor.inbox.post(&closed, 0).unwrap();
        assert!(monitor.drain(Selection::All).unwrap().is_empty());
    }
}
