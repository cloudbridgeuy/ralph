use crate::chat::inbox::Display;
use crate::chat::schedule::{ask_answer, read_answer};
use crate::chat::thread::{
    config_root, default_listing, listing, merge_precheck, merge_thread, park_thread, pull_thread,
    rows, thread_dir, view_thread,
};
use crate::cli::MERGE_TIMEOUT_SECS;
use crate::{run_curator, Curator};
use ralph_core::chat_memory::CallSite;
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_cmd::{
    merge_answer, merge_question, merging_notice, op_refusal, parked_message, view_posted_message,
    ThreadOp,
};
use std::io;
use std::path::Path;

pub(super) struct ThreadEnv<'a> {
    pub store_dir: &'a Path,
    pub in_thread: bool,
    pub display: Option<&'a Display<'a>>,
}

pub(super) fn run(
    op: &ThreadOp,
    env: &ThreadEnv,
    confirm: &mut impl FnMut(&str) -> Option<bool>,
    running_of: &impl Fn(&ThreadName) -> Vec<String>,
) -> Result<String, String> {
    if let Some(refusal) = op_refusal(env.in_thread, op) {
        return Err(refusal);
    }
    let main = config_root(env.store_dir);
    match op {
        ThreadOp::Ls => {
            let listed = rows(&main, &|name| running_of(name).len())
                .map_err(|e| format!("could not list threads: {e}"))?;
            Ok(match env.display {
                Some(display) => listing(&listed, display, "no threads"),
                None => default_listing(&listed, "no threads"),
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
                timeout: MERGE_TIMEOUT_SECS,
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
    use crate::chat::thread::ensure;
    use crate::highlight::Highlighter;
    use crate::markdown::MarkdownTheme;
    use crate::render::RenderContext;
    use crate::ui_theme::UiColors;
    use ralph_server::chat::memory::Store;

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

    fn env(main: &Path, in_thread: bool) -> ThreadEnv<'_> {
        ThreadEnv {
            store_dir: main,
            in_thread,
            display: None,
        }
    }

    #[test]
    fn ls_in_a_chat_prints_a_table_with_the_chat_display() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let highlighter = Highlighter::new();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::new(&highlighter, true),
            theme: &theme,
            color: true,
            ui: UiColors::default(),
        };
        let env = ThreadEnv {
            display: Some(&display),
            ..env(&main, false)
        };
        let text = run(&ThreadOp::Ls, &env, &mut |_: &str| None, &idle).unwrap();
        assert!(text.contains('│') && text.contains("auth"), "{text}");
        assert!(!text.contains("| NAME |"), "{text}");
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
        let result = run(&merge_auth(), &env(&main, false), &mut confirm, &idle);
        assert_eq!(result, Ok("merge cancelled.".to_string()));
        assert_eq!(asked, 1);
    }

    #[test]
    fn an_aborted_merge_question_is_refused_not_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let refusal = run(
            &merge_auth(),
            &env(&main, false),
            &mut |_: &str| None,
            &idle,
        )
        .unwrap_err();
        assert!(refusal.starts_with("merge aborted"), "{refusal}");
    }

    #[test]
    fn merging_an_unknown_thread_refuses_without_asking() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let refusal = run(&merge_auth(), &env(&main, false), &mut confirm, &idle).unwrap_err();
        assert!(refusal.starts_with("no thread auth"), "{refusal}");
    }

    #[test]
    fn merging_from_a_thread_refuses_without_asking() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let refusal = run(&merge_auth(), &env(&main, true), &mut confirm, &idle).unwrap_err();
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
            &env(&main, false),
            &mut |_: &str| Some(true),
            &running_of,
        )
        .unwrap_err();
        assert!(refusal.contains("running workers (w1)"), "{refusal}");
        assert_eq!(lookups.get(), 2);
    }
}
