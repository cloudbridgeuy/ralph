use crate::chat_help::Command;
use crate::chat_switch::SwitchTarget;
use crate::chat_thread::ThreadName;

pub const VIEW_FLAG: &str = "--view";
const VIEW_ALIAS: &str = "--rebase";
const LS_USAGE: &str = "usage: /thread ls takes no arguments.";
const MAIN_USAGE: &str = "usage: /thread main takes no arguments and switches this chat to main.";
const SWITCH_USAGE: &str =
    "usage: /thread switch <name> switches this chat to thread <name>; /thread main returns to main.";
const PARK_USAGE: &str =
    "usage: /thread park <name> stops thread <name> from opening; run it in main's chat.";
const PULL_USAGE: &str =
    "usage: /thread pull <name> moves thread <name> to main's end; run it in main's chat.";
const REVIVE_USAGE: &str =
    "usage: /thread revive <name> lets parked thread <name> open again; run it in main's chat.";
const DROP_USAGE: &str =
    "usage: /thread drop <name> deletes thread <name> after a y/n; run it in main's chat.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOp {
    Ls,
    Pull(ThreadName),
    Park(ThreadName),
    Revive(ThreadName),
    Drop(ThreadName),
    Merge { name: ThreadName, view: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadLine {
    Switch(Result<SwitchTarget, String>),
    Op(Result<ThreadOp, String>),
}

pub fn parse_thread_args(args: &str) -> ThreadLine {
    let words: Vec<&str> = args.split_whitespace().collect();
    match words.split_first() {
        Some((&"ls", [])) => ThreadLine::Op(Ok(ThreadOp::Ls)),
        Some((&"ls", _)) => ThreadLine::Op(Err(LS_USAGE.to_string())),
        Some((&"main", [])) => ThreadLine::Switch(Ok(SwitchTarget::Main)),
        Some((&"main", _)) => ThreadLine::Switch(Err(MAIN_USAGE.to_string())),
        Some((&"switch", rest)) => {
            ThreadLine::Switch(one_name(rest, SWITCH_USAGE).map(SwitchTarget::Thread))
        }
        Some((&"pull", rest)) => ThreadLine::Op(one_name(rest, PULL_USAGE).map(ThreadOp::Pull)),
        Some((&"park", rest)) => ThreadLine::Op(one_name(rest, PARK_USAGE).map(ThreadOp::Park)),
        Some((&"revive", rest)) => {
            ThreadLine::Op(one_name(rest, REVIVE_USAGE).map(ThreadOp::Revive))
        }
        Some((&"drop", rest)) => ThreadLine::Op(one_name(rest, DROP_USAGE).map(ThreadOp::Drop)),
        Some((&"merge", rest)) => ThreadLine::Op(parse_merge(rest)),
        _ => ThreadLine::Switch(Err(thread_usage())),
    }
}

pub fn op_refusal(in_thread: bool, op: &ThreadOp) -> Option<String> {
    match op {
        ThreadOp::Ls => None,
        ThreadOp::Pull(_) => in_thread.then(|| main_only("pull")),
        ThreadOp::Park(_) => in_thread.then(|| main_only("park")),
        ThreadOp::Revive(_) => in_thread.then(|| main_only("revive")),
        ThreadOp::Drop(_) => in_thread.then(|| main_only("drop")),
        ThreadOp::Merge { .. } => in_thread.then(|| main_only("merge")),
    }
}

fn thread_usage() -> String {
    format!(
        "usage: /thread <ls | switch <name> | main | pull <name> | park <name> | revive <name> | drop <name> | merge <name> [{VIEW_FLAG}]>"
    )
}

fn merge_usage() -> String {
    format!(
        "usage: /thread merge <name> [{VIEW_FLAG}] merges thread <name> into main's inbox; {VIEW_FLAG} posts its memory view with no model call and no confirmation; run it in main's chat."
    )
}

fn parse_merge(words: &[&str]) -> Result<ThreadOp, String> {
    let (flags, rest): (Vec<&str>, Vec<&str>) = words
        .iter()
        .partition(|word| **word == VIEW_FLAG || **word == VIEW_ALIAS);
    match (flags.len(), rest.as_slice()) {
        (0 | 1, [word]) => ThreadName::parse(word)
            .map(|name| ThreadOp::Merge {
                name,
                view: !flags.is_empty(),
            })
            .map_err(|e| format!("{e}\n{}", merge_usage())),
        _ => Err(merge_usage()),
    }
}

fn one_name(words: &[&str], usage: &str) -> Result<ThreadName, String> {
    match words {
        [word] => ThreadName::parse(word).map_err(|e| format!("{e}\n{usage}")),
        _ => Err(usage.to_string()),
    }
}

pub fn view_posted_message(name: &ThreadName, report: &str) -> String {
    format!("posted thread {name}'s memory view to main's inbox.\n{report}")
}

pub fn parked_message(name: &ThreadName) -> String {
    format!("parked thread {name}; `ralph thread revive {name}` opens it again")
}

pub fn revived_message(name: &ThreadName) -> String {
    format!("revived thread {name}")
}

pub fn drop_question(name: &ThreadName) -> String {
    format!("Drop thread {name}? This deletes its folder and cannot be undone. [y/n] ")
}

pub fn drop_answer(answer: Option<bool>) -> Result<bool, String> {
    answer.ok_or_else(|| "drop aborted; the command stays in the prompt.".to_string())
}

pub fn is_cell_command(line: &str) -> bool {
    Command::Thread.args(line).is_some()
}

pub fn merging_notice(name: &ThreadName) -> String {
    format!("merging {name} (one paid Haiku call, up to 3 if the digest is refused)...")
}

pub fn merge_question(name: &ThreadName) -> String {
    format!(
        "Merge thread {name} into main? This makes one paid Haiku call, and up to 3 if the digest is refused. [y/n] "
    )
}

pub fn merge_answer(answer: Option<bool>) -> Result<bool, String> {
    answer.ok_or_else(|| "merge aborted; the command stays in the prompt.".to_string())
}

fn main_only(sub: &str) -> String {
    format!("/thread {sub} runs in main's chat. Run /thread main, then /thread {sub} <name>.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth() -> ThreadName {
        ThreadName::parse("auth").unwrap()
    }

    fn op(args: &str) -> Result<ThreadOp, String> {
        match parse_thread_args(args) {
            ThreadLine::Op(parsed) => parsed,
            other => panic!("{args:?} is not a thread op: {other:?}"),
        }
    }

    fn merge(view: bool) -> ThreadOp {
        ThreadOp::Merge { name: auth(), view }
    }

    #[test]
    fn only_an_answer_decides_the_merge() {
        assert_eq!(merge_answer(Some(true)), Ok(true));
        assert_eq!(merge_answer(Some(false)), Ok(false));
        let aborted = merge_answer(None).unwrap_err();
        assert!(aborted.starts_with("merge aborted"), "{aborted}");
    }

    #[test]
    fn ls_takes_no_arguments() {
        assert_eq!(op(" ls "), Ok(ThreadOp::Ls));
        assert_eq!(op("ls auth"), Err(LS_USAGE.to_string()));
    }

    #[test]
    fn pull_and_park_take_one_thread_name() {
        assert_eq!(op(" pull auth "), Ok(ThreadOp::Pull(auth())));
        assert_eq!(op("park auth"), Ok(ThreadOp::Park(auth())));
    }

    #[test]
    fn pull_and_park_refuse_no_name_two_names_or_a_bad_name_with_usage() {
        for (sub, usage) in [("pull", PULL_USAGE), ("park", PARK_USAGE)] {
            assert_eq!(op(sub), Err(usage.to_string()));
            assert_eq!(op(&format!("{sub} auth now")), Err(usage.to_string()));
            let bad = op(&format!("{sub} Bad_Name")).unwrap_err();
            assert!(bad.starts_with("'Bad_Name' is not a thread name."), "{bad}");
            assert!(bad.ends_with(usage), "{bad}");
        }
    }

    #[test]
    fn revive_and_drop_take_one_thread_name() {
        assert_eq!(op(" revive auth "), Ok(ThreadOp::Revive(auth())));
        assert_eq!(op("drop auth"), Ok(ThreadOp::Drop(auth())));
    }

    #[test]
    fn revive_and_drop_refuse_no_name_two_names_or_a_bad_name_with_usage() {
        for (sub, usage) in [("revive", REVIVE_USAGE), ("drop", DROP_USAGE)] {
            assert_eq!(op(sub), Err(usage.to_string()));
            assert_eq!(op(&format!("{sub} auth now")), Err(usage.to_string()));
            let bad = op(&format!("{sub} Bad_Name")).unwrap_err();
            assert!(bad.ends_with(usage), "{bad}");
        }
    }

    #[test]
    fn merge_squashes_without_the_flag_and_posts_the_view_with_it() {
        assert_eq!(op("merge  auth "), Ok(merge(false)));
        for line in [
            "merge auth --view",
            "merge --view  auth",
            "merge auth --rebase",
            "merge --rebase auth",
        ] {
            assert_eq!(op(line), Ok(merge(true)), "{line}");
        }
    }

    #[test]
    fn merge_with_the_wrong_shape_gets_the_usage() {
        for line in [
            "merge",
            "merge --view",
            "merge auth other",
            "merge auth --view --rebase",
            "merge auth --view --view",
            "merge auth --force",
        ] {
            let err = op(line).unwrap_err();
            assert!(
                err.starts_with("usage: /thread merge <name> [--view]"),
                "{line}: {err}"
            );
        }
        let bad = op("merge Bad_Name").unwrap_err();
        assert!(bad.starts_with("'Bad_Name' is not a thread name."), "{bad}");
    }

    #[test]
    fn switch_takes_one_thread_name_and_never_main() {
        assert_eq!(
            parse_thread_args("switch auth"),
            ThreadLine::Switch(Ok(SwitchTarget::Thread(auth())))
        );
        for line in ["switch", "switch a b", "switch Bad_Name", "switch main"] {
            let ThreadLine::Switch(Err(err)) = parse_thread_args(line) else {
                panic!("{line}: not a switch refusal");
            };
            assert!(err.ends_with(SWITCH_USAGE), "{line}: {err}");
        }
    }

    #[test]
    fn main_takes_no_arguments() {
        assert_eq!(
            parse_thread_args(" main "),
            ThreadLine::Switch(Ok(SwitchTarget::Main))
        );
        assert_eq!(
            parse_thread_args("main side"),
            ThreadLine::Switch(Err(MAIN_USAGE.to_string()))
        );
    }

    #[test]
    fn no_subcommand_or_a_bare_name_gets_the_general_usage() {
        for line in ["", "  ", "auth", "Merge auth"] {
            assert_eq!(
                parse_thread_args(line),
                ThreadLine::Switch(Err(thread_usage())),
                "{line:?}"
            );
        }
    }

    #[test]
    fn subcommand_words_are_not_reserved_as_thread_names() {
        for sub in ["ls", "drop", "switch", "merge", "park", "pull"] {
            let name = ThreadName::parse(sub).unwrap();
            assert_eq!(
                parse_thread_args(&format!("switch {sub}")),
                ThreadLine::Switch(Ok(SwitchTarget::Thread(name.clone())))
            );
            assert_eq!(op(&format!("park {sub}")), Ok(ThreadOp::Park(name)));
        }
    }

    #[test]
    fn a_thread_chat_refuses_the_main_only_ops_with_a_hint_and_main_allows_them() {
        for (op, sub) in [
            (ThreadOp::Pull(auth()), "pull"),
            (ThreadOp::Park(auth()), "park"),
            (ThreadOp::Revive(auth()), "revive"),
            (ThreadOp::Drop(auth()), "drop"),
            (merge(false), "merge"),
            (merge(true), "merge"),
        ] {
            assert_eq!(
                op_refusal(true, &op),
                Some(format!(
                    "/thread {sub} runs in main's chat. Run /thread main, then /thread {sub} <name>."
                ))
            );
            assert_eq!(op_refusal(false, &op), None);
        }
    }

    #[test]
    fn ls_is_never_refused() {
        assert_eq!(op_refusal(true, &ThreadOp::Ls), None);
        assert_eq!(op_refusal(false, &ThreadOp::Ls), None);
    }

    #[test]
    fn the_merge_question_names_the_thread_the_cost_and_the_answer_keys() {
        let question = merge_question(&auth());
        assert!(question.contains("auth"), "{question}");
        assert!(question.contains("Haiku"), "{question}");
        assert!(question.ends_with("[y/n] "), "{question}");
    }

    #[test]
    fn only_an_answer_decides_the_drop() {
        assert_eq!(drop_answer(Some(true)), Ok(true));
        assert_eq!(drop_answer(Some(false)), Ok(false));
        let aborted = drop_answer(None).unwrap_err();
        assert!(aborted.starts_with("drop aborted"), "{aborted}");
    }

    #[test]
    fn the_drop_question_names_the_thread_and_that_it_cannot_be_undone() {
        let question = drop_question(&auth());
        assert!(question.contains("auth"), "{question}");
        assert!(question.contains("cannot be undone"), "{question}");
        assert!(question.ends_with("[y/n] "), "{question}");
    }

    #[test]
    fn the_revived_message_names_the_thread() {
        assert_eq!(revived_message(&auth()), "revived thread auth");
    }

    #[test]
    fn the_view_posted_message_names_the_thread_and_keeps_the_report() {
        assert_eq!(
            view_posted_message(&auth(), "[thread auth · view]\nblock"),
            "posted thread auth's memory view to main's inbox.\n[thread auth · view]\nblock"
        );
    }

    #[test]
    fn the_parked_message_names_the_thread_and_the_way_back() {
        assert_eq!(
            parked_message(&auth()),
            "parked thread auth; `ralph thread revive auth` opens it again"
        );
    }

    #[test]
    fn the_merging_notice_names_the_thread_and_the_retry_cost() {
        let notice = merging_notice(&auth());
        assert!(notice.contains("auth"), "{notice}");
        assert!(
            notice.contains("up to 3 if the digest is refused"),
            "{notice}"
        );
    }
}
