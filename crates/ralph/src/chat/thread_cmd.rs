use crate::chat::inbox::Display;
use crate::chat::schedule::{ask_answer, read_answer};
use crate::chat::thread::{
    config_root, drop_precheck, drop_thread, listing, merge_precheck, merge_thread, park_thread,
    pull_thread, revive_thread, rows, thread_dir, view_thread,
};
use crate::cli::MERGE_TIMEOUT_SECS;
use crate::{run_curator, Curator};
use ralph_core::chat_local::local_record;
use ralph_core::chat_memory::CallSite;
use ralph_core::chat_memory::Role;
use ralph_core::chat_thread::ThreadName;
use ralph_core::chat_thread_cmd::{
    drop_answer, drop_question, merge_answer, merge_posted_message, merge_question, merging_notice,
    op_refusal, parked_message, revived_message, view_posted_message, ThreadOp, VIEW_FLAG,
};
use ralph_server::chat::memory::Store;
use ralph_server::warn::warn_if_err;
use std::io;
use std::path::Path;

pub(super) struct ThreadEnv<'a> {
    pub store_dir: &'a Path,
    pub in_thread: bool,
    pub width: usize,
    pub display: &'a Display<'a>,
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
            Ok(listing(&listed, env.display, env.width, "no threads"))
        }
        ThreadOp::Pull(name) => pull_thread(&main, name),
        ThreadOp::Park(name) => {
            park_thread(&main, name, &running_of(name)).map(|()| parked_message(name))
        }
        ThreadOp::Revive(name) => revive_thread(&main, name).map(|()| revived_message(name)),
        ThreadOp::Drop(name) => {
            drop_precheck(&main, name, &running_of(name))?;
            if !drop_answer(confirm(&drop_question(name)))? {
                return Ok("drop cancelled.".to_string());
            }
            drop_thread(&main, name, &running_of(name))
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
            .map(|message| merge_posted_message(name, &message))
        }
    }
}

pub(super) fn merge_line(name: &ThreadName, view: bool) -> String {
    match view {
        true => format!("/thread merge {name} {VIEW_FLAG}"),
        false => format!("/thread merge {name}"),
    }
}

pub(super) fn record_cli_merge(main: &Path, name: &ThreadName, line: &str, output: &str) {
    warn_if_err(
        Store::new(main.to_path_buf()).append(Role::User, &local_record(line, output)),
        format!("Failed to record the thread merge; the digest is in threads/{name}/thread.json under the store"),
    );
}

pub(super) fn confirm(question: &str) -> Option<bool> {
    ask_answer(|| question.to_string(), &mut read_answer, &mut io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ansi::fg;
    use crate::chat::thread::ensure;
    use crate::chat::workers::{paint_tones, table_theme};
    use crate::highlight::Highlighter;
    use crate::markdown::MarkdownTheme;
    use crate::render::text::render_prose;
    use crate::render::RenderContext;
    use crate::ui_theme::UiColors;
    use ralph_core::chat_thread_table::{threads_view, ThreadTableOpts};
    use ralph_core::workers_view::View;

    #[test]
    fn the_merge_line_names_the_thread_and_the_view_flag() {
        let auth = ThreadName::parse("auth").unwrap();
        assert_eq!(merge_line(&auth, false), "/thread merge auth");
        assert_eq!(merge_line(&auth, true), "/thread merge auth --view");
    }

    #[test]
    fn a_cli_merge_appends_one_row_with_the_digest() {
        let auth = ThreadName::parse("auth").unwrap();
        for view in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let main = root.path().join("chat");
            let before = Store::new(main.clone()).log_len().unwrap();
            let line = merge_line(&auth, view);
            record_cli_merge(
                &main,
                &auth,
                &line,
                "merged thread auth\n[digest] Picked x. #auth:0",
            );
            let store = Store::new(main.clone());
            assert_eq!(store.log_len().unwrap(), before + 1);
            let (entries, _) = store.load().unwrap();
            let row = &entries[before].text;
            assert!(row.starts_with(&line), "{row}");
            assert!(row.contains("Picked x. #auth:0"), "{row}");
        }
    }

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

    fn with_env<R>(main: &Path, in_thread: bool, check: impl FnOnce(&ThreadEnv) -> R) -> R {
        let highlighter = Highlighter::new();
        let theme = MarkdownTheme::default();
        let display = Display {
            ctx: RenderContext::new(&highlighter, true),
            theme: &theme,
            color: true,
            ui: UiColors::default(),
        };
        check(&ThreadEnv {
            store_dir: main,
            in_thread,
            width: 200,
            display: &display,
        })
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
            store_dir: &main,
            in_thread: false,
            width: 200,
            display: &display,
        };
        let text = run(&ThreadOp::Ls, &env, &mut |_: &str| None, &idle).unwrap();
        // The in-chat prose wraps to the terminal's own width, so check only the border.
        assert!(text.contains('│'), "{text}");
        assert!(!text.contains("| NAME |"), "{text}");
        // Paint the same table at a fixed width, as the /workers painted test does.
        let opts = ThreadTableOpts {
            now: chrono::Utc::now(),
            width: 200,
            styled: true,
        };
        let listed = rows(&main, &|_: &ThreadName| 0).unwrap();
        let Some(View::Table(md)) = threads_view(&listed, &opts) else {
            panic!("not a table");
        };
        let skin = table_theme(&theme).skin;
        let painted = paint_tones(&render_prose(&skin, &md, 200), &theme.palette);
        assert!(painted.contains("NAME"), "{painted}");
        assert!(painted.contains("auth"), "{painted}");
        assert!(
            painted.contains(&format!("{}open", fg(theme.palette.done))),
            "{painted:?}"
        );
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
        let result = with_env(&main, false, |env| {
            run(&merge_auth(), env, &mut confirm, &idle)
        });
        assert_eq!(result, Ok("merge cancelled.".to_string()));
        assert_eq!(asked, 1);
    }

    #[test]
    fn an_aborted_merge_question_is_refused_not_cancelled() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let refusal = with_env(&main, false, |env| {
            run(&merge_auth(), env, &mut |_: &str| None, &idle)
        })
        .unwrap_err();
        assert!(refusal.starts_with("merge aborted"), "{refusal}");
    }

    #[test]
    fn merging_an_unknown_thread_refuses_without_asking() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let refusal = with_env(&main, false, |env| {
            run(&merge_auth(), env, &mut confirm, &idle)
        })
        .unwrap_err();
        assert!(refusal.starts_with("no thread auth"), "{refusal}");
    }

    #[test]
    fn merging_from_a_thread_refuses_without_asking() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let refusal = with_env(&main, true, |env| {
            run(&merge_auth(), env, &mut confirm, &idle)
        })
        .unwrap_err();
        assert!(
            refusal.contains("Run /thread main, then /thread merge <name>."),
            "{refusal}"
        );
    }

    fn drop_auth() -> ThreadOp {
        ThreadOp::Drop(ThreadName::parse("auth").unwrap())
    }

    #[test]
    fn a_confirmed_drop_deletes_the_thread_after_one_question() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let auth = ThreadName::parse("auth").unwrap();
        ensure(&main, &auth).unwrap();
        let mut asked = 0;
        let mut confirm = |question: &str| {
            asked += 1;
            assert!(question.contains("cannot be undone"), "{question}");
            Some(true)
        };
        let result = with_env(&main, false, |env| {
            run(&drop_auth(), env, &mut confirm, &idle)
        });
        assert_eq!(result, Ok("dropped thread auth".to_string()));
        assert_eq!(asked, 1);
        assert!(!thread_dir(&main, &auth).exists());
    }

    #[test]
    fn a_declined_drop_keeps_the_thread() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let auth = ThreadName::parse("auth").unwrap();
        ensure(&main, &auth).unwrap();
        let result = with_env(&main, false, |env| {
            run(&drop_auth(), env, &mut |_: &str| Some(false), &idle)
        });
        assert_eq!(result, Ok("drop cancelled.".to_string()));
        assert!(thread_dir(&main, &auth).exists());
    }

    #[test]
    fn an_aborted_drop_question_is_refused_and_keeps_the_thread() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let auth = ThreadName::parse("auth").unwrap();
        ensure(&main, &auth).unwrap();
        let refusal = with_env(&main, false, |env| {
            run(&drop_auth(), env, &mut |_: &str| None, &idle)
        })
        .unwrap_err();
        assert!(refusal.starts_with("drop aborted"), "{refusal}");
        assert!(thread_dir(&main, &auth).exists());
    }

    #[test]
    fn a_drop_that_the_thread_state_refuses_never_asks() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let unknown = with_env(&main, false, |env| {
            run(&drop_auth(), env, &mut confirm, &idle)
        })
        .unwrap_err();
        assert!(unknown.starts_with("no thread auth"), "{unknown}");
        ensure(&main, &ThreadName::parse("auth").unwrap()).unwrap();
        let busy = with_env(&main, false, |env| {
            run(&drop_auth(), env, &mut confirm, &|_: &ThreadName| {
                vec!["w1".to_string()]
            })
        })
        .unwrap_err();
        assert!(busy.contains("running workers (w1)"), "{busy}");
        let in_thread = with_env(&main, true, |env| {
            run(&drop_auth(), env, &mut confirm, &idle)
        })
        .unwrap_err();
        assert!(
            in_thread.contains("Run /thread main, then /thread drop <name>."),
            "{in_thread}"
        );
    }

    #[test]
    fn revive_opens_a_parked_thread_and_refuses_one_that_is_not_parked() {
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("chat");
        let auth = ThreadName::parse("auth").unwrap();
        ensure(&main, &auth).unwrap();
        let revive = ThreadOp::Revive(auth.clone());
        let mut confirm = |_: &str| -> Option<bool> { panic!("must not ask") };
        let not_parked =
            with_env(&main, false, |env| run(&revive, env, &mut confirm, &idle)).unwrap_err();
        assert_eq!(not_parked, "thread auth is not parked.");
        let park = ThreadOp::Park(auth);
        with_env(&main, false, |env| run(&park, env, &mut confirm, &idle)).unwrap();
        let revived = with_env(&main, false, |env| run(&revive, env, &mut confirm, &idle));
        assert_eq!(revived, Ok("revived thread auth".to_string()));
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
        let refusal = with_env(&main, false, |env| {
            run(&merge_auth(), env, &mut |_: &str| Some(true), &running_of)
        })
        .unwrap_err();
        assert!(refusal.contains("running workers (w1)"), "{refusal}");
        assert_eq!(lookups.get(), 2);
    }
}
