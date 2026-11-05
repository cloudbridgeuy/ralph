use crate::chat_help::Command;
use crate::chat_thread::ThreadName;

pub const VIEW_FLAG: &str = "--view";
const VIEW_ALIAS: &str = "--rebase";
const PARK_USAGE: &str =
    "usage: /park <name> stops thread <name> from opening; run it in main's chat.";
const PULL_USAGE: &str =
    "usage: /pull <name> moves thread <name> to main's end; run it in main's chat.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOp {
    Ls,
    Pull(ThreadName),
    Park(ThreadName),
    Merge { name: ThreadName, view: bool },
}

pub fn parse_thread_op(command: Command, args: &str) -> Option<Result<ThreadOp, String>> {
    if args.trim().is_empty() {
        return match command {
            Command::Threads => Some(Ok(ThreadOp::Ls)),
            Command::Pull => Some(Err(PULL_USAGE.to_string())),
            Command::Park => Some(Err(PARK_USAGE.to_string())),
            Command::Merge => Some(Err(merge_usage())),
            _ => None,
        };
    }
    match command {
        Command::Pull => thread_arg(args).map(ThreadOp::Pull),
        Command::Park => thread_arg(args).map(ThreadOp::Park),
        Command::Merge => parse_merge(args),
        _ => None,
    }
    .map(Ok)
}

pub fn op_refusal(in_thread: bool, op: &ThreadOp) -> Option<String> {
    match op {
        ThreadOp::Ls => None,
        ThreadOp::Pull(_) => in_thread.then(|| main_only(Command::Pull)),
        ThreadOp::Park(_) => in_thread.then(|| main_only(Command::Park)),
        ThreadOp::Merge { .. } => in_thread.then(|| main_only(Command::Merge)),
    }
}

fn merge_usage() -> String {
    format!(
        "usage: /merge <name> [{VIEW_FLAG}] merges thread <name> into main's inbox; {VIEW_FLAG} posts its memory view with no model call and no confirmation; run it in main's chat."
    )
}

fn parse_merge(args: &str) -> Option<ThreadOp> {
    let (flags, rest): (Vec<&str>, Vec<&str>) = args
        .split_whitespace()
        .partition(|word| *word == VIEW_FLAG || *word == VIEW_ALIAS);
    match (flags.len(), rest.as_slice()) {
        (0 | 1, [word]) => ThreadName::parse(word).ok().map(|name| ThreadOp::Merge {
            name,
            view: !flags.is_empty(),
        }),
        _ => None,
    }
}

pub fn view_posted_message(name: &ThreadName, report: &str) -> String {
    format!("posted thread {name}'s memory view to main's inbox.\n{report}")
}

pub fn parked_message(name: &ThreadName) -> String {
    format!("parked thread {name}; `ralph thread revive {name}` opens it again")
}

pub fn is_cell_command(line: &str) -> bool {
    let plain = [Command::Thread, Command::Main]
        .iter()
        .any(|command| command.args(line).is_some());
    let shaped = [
        Command::Threads,
        Command::Pull,
        Command::Park,
        Command::Merge,
    ]
    .iter()
    .any(|command| {
        command
            .args(line)
            .is_some_and(|args| parse_thread_op(*command, args).is_some())
    });
    plain || shaped
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

fn main_only(command: Command) -> String {
    let name = command.name();
    format!("{name} runs in main's chat. Run /main, then {name} <name>.")
}

fn thread_arg(args: &str) -> Option<ThreadName> {
    let mut words = args.split_whitespace();
    match (words.next(), words.next()) {
        (Some(word), None) => ThreadName::parse(word).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_help::Command;
    use crate::chat_thread::ThreadName;

    #[test]
    fn only_an_answer_decides_the_merge() {
        assert_eq!(merge_answer(Some(true)), Ok(true));
        assert_eq!(merge_answer(Some(false)), Ok(false));
        let aborted = merge_answer(None).unwrap_err();
        assert!(aborted.starts_with("merge aborted"), "{aborted}");
    }

    #[test]
    fn threads_without_arguments_lists() {
        assert_eq!(
            parse_thread_op(Command::Threads, ""),
            Some(Ok(ThreadOp::Ls))
        );
        assert_eq!(
            parse_thread_op(Command::Threads, "  "),
            Some(Ok(ThreadOp::Ls))
        );
    }

    #[test]
    fn threads_with_an_argument_is_not_a_thread_op() {
        assert_eq!(parse_thread_op(Command::Threads, " auth"), None);
    }

    #[test]
    fn a_command_that_is_not_a_thread_command_is_not_a_thread_op() {
        assert_eq!(parse_thread_op(Command::Help, ""), None);
        assert_eq!(parse_thread_op(Command::Help, "auth"), None);
    }

    fn auth() -> ThreadName {
        ThreadName::parse("auth").unwrap()
    }

    #[test]
    fn pull_takes_one_thread_name() {
        assert_eq!(
            parse_thread_op(Command::Pull, " auth "),
            Some(Ok(ThreadOp::Pull(auth())))
        );
    }

    #[test]
    fn pull_without_a_name_is_a_usage_error() {
        let err = parse_thread_op(Command::Pull, "  ").unwrap().unwrap_err();
        assert!(err.starts_with("usage: /pull <name>"), "{err}");
    }

    #[test]
    fn pull_with_extra_words_or_a_bad_name_is_not_a_thread_op() {
        for args in ["auth now", "auth\nfix", "Bad_Name"] {
            assert_eq!(parse_thread_op(Command::Pull, args), None, "{args}");
        }
    }

    #[test]
    fn a_thread_chat_refuses_pull_with_a_hint_and_main_allows_it() {
        let pull = ThreadOp::Pull(auth());
        assert_eq!(
            op_refusal(true, &pull),
            Some("/pull runs in main's chat. Run /main, then /pull <name>.".to_string())
        );
        assert_eq!(op_refusal(false, &pull), None);
    }

    #[test]
    fn ls_is_never_refused() {
        assert_eq!(op_refusal(true, &ThreadOp::Ls), None);
        assert_eq!(op_refusal(false, &ThreadOp::Ls), None);
    }

    #[test]
    fn route_input_routes_pull_and_it_queues_mid_turn() {
        use crate::chat_status::{route_input, EmptySubmit, Route};
        use std::collections::BTreeSet;
        let routed = route_input("/pull auth", 0, &EmptySubmit::default(), &BTreeSet::new());
        assert_eq!(
            routed,
            Route::ThreadOp {
                op: Ok(ThreadOp::Pull(auth())),
                line: "/pull auth".into()
            }
        );
        assert!(!routed.runs_mid_turn());
    }

    #[test]
    fn park_takes_one_thread_name() {
        assert_eq!(
            parse_thread_op(Command::Park, "auth"),
            Some(Ok(ThreadOp::Park(auth())))
        );
    }

    #[test]
    fn park_without_a_name_is_a_usage_error() {
        let err = parse_thread_op(Command::Park, "").unwrap().unwrap_err();
        assert!(err.starts_with("usage: /park <name>"), "{err}");
    }

    #[test]
    fn park_with_extra_words_is_not_a_thread_op() {
        assert_eq!(parse_thread_op(Command::Park, "auth now"), None);
    }

    #[test]
    fn a_thread_chat_refuses_park_with_a_hint_and_main_allows_it() {
        let park = ThreadOp::Park(auth());
        assert_eq!(
            op_refusal(true, &park),
            Some("/park runs in main's chat. Run /main, then /park <name>.".to_string())
        );
        assert_eq!(op_refusal(false, &park), None);
    }

    fn merge(view: bool) -> ThreadOp {
        ThreadOp::Merge { name: auth(), view }
    }

    #[test]
    fn merge_without_the_flag_squashes() {
        assert_eq!(
            parse_thread_op(Command::Merge, " auth "),
            Some(Ok(merge(false)))
        );
    }

    #[test]
    fn merge_with_the_rebase_alias_in_either_place_posts_the_view() {
        for line in ["auth --rebase", "--rebase  auth"] {
            assert_eq!(
                parse_thread_op(Command::Merge, line),
                Some(Ok(merge(true))),
                "{line}"
            );
        }
    }

    #[test]
    fn merge_with_the_view_flag_in_either_place_posts_the_view() {
        for line in ["auth --view", "--view  auth"] {
            assert_eq!(
                parse_thread_op(Command::Merge, line),
                Some(Ok(merge(true))),
                "{line}"
            );
        }
    }

    #[test]
    fn merge_with_the_view_flag_and_its_alias_is_a_repeat() {
        for line in ["auth --view --rebase", "auth --view --view"] {
            assert_eq!(parse_thread_op(Command::Merge, line), None, "{line}");
        }
    }

    #[test]
    fn merge_without_arguments_is_a_usage_error() {
        let err = parse_thread_op(Command::Merge, "").unwrap().unwrap_err();
        assert!(err.starts_with("usage: /merge <name> [--view]"), "{err}");
        assert!(err.contains(&format!("[{VIEW_FLAG}]")), "{err}");
    }

    #[test]
    fn merge_with_the_wrong_shape_is_not_a_thread_op() {
        let both = format!("auth other {VIEW_FLAG}");
        let twice = format!("auth {VIEW_FLAG} {VIEW_FLAG}");
        for args in [VIEW_FLAG, "auth other", "Bad_Name", &both, &twice] {
            assert_eq!(parse_thread_op(Command::Merge, args), None, "{args}");
        }
    }

    #[test]
    fn a_thread_chat_refuses_both_merges_with_a_hint_and_main_allows_them() {
        for view in [false, true] {
            assert_eq!(
                op_refusal(true, &merge(view)),
                Some("/merge runs in main's chat. Run /main, then /merge <name>.".to_string())
            );
            assert_eq!(op_refusal(false, &merge(view)), None);
        }
    }

    #[test]
    fn the_merge_question_names_the_thread_the_cost_and_the_answer_keys() {
        let question = merge_question(&auth());
        assert!(question.contains("auth"), "{question}");
        assert!(question.contains("Haiku"), "{question}");
        assert!(question.ends_with("[y/n] "), "{question}");
    }

    #[test]
    fn route_input_routes_threads_and_it_queues_mid_turn() {
        use crate::chat_status::{route_input, EmptySubmit, Route};
        use std::collections::BTreeSet;
        let route = |line: &str| route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new());
        let routed = route("/threads");
        assert_eq!(
            routed,
            Route::ThreadOp {
                op: Ok(ThreadOp::Ls),
                line: "/threads".into()
            }
        );
        assert!(!routed.runs_mid_turn());
        assert!(matches!(route("/threads x"), Route::Send(_)));
        assert!(matches!(route("/threadsx"), Route::Send(_)));
        assert!(matches!(route("/thread auth"), Route::Switch { .. }));
    }

    #[test]
    fn lines_that_do_not_have_the_command_shape_go_to_the_model() {
        use crate::chat_status::{route_input, EmptySubmit, Route};
        use std::collections::BTreeSet;
        let route = |line: &str| route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new());
        for line in [
            "/merge requests are failing on CI",
            "/pull Bad_Name",
            "/park auth now",
            "/merge --view",
            "/merge auth --force",
            "/merge auth --view --view",
            "/threads x",
        ] {
            assert_eq!(route(line), Route::Send(line.to_string()), "{line}");
        }
    }

    #[test]
    fn well_formed_and_bare_commands_stay_thread_ops() {
        use crate::chat_status::{route_input, EmptySubmit, Route};
        use std::collections::BTreeSet;
        let route = |line: &str| route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new());
        for line in ["/pull", "/park", "/merge"] {
            assert!(
                matches!(route(line), Route::ThreadOp { op: Err(_), .. }),
                "{line}"
            );
        }
        assert!(matches!(
            route("/merge auth --view"),
            Route::ThreadOp { op: Ok(_), .. }
        ));
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
    fn thread_and_switch_commands_are_cell_commands() {
        for line in [
            "/merge a",
            "/threads",
            "/pull a",
            "/park a",
            "/thread a",
            "/main",
            "/merge a --view",
            "/merge",
            "/pull",
            "/park",
            "/thread a b",
        ] {
            assert!(is_cell_command(line), "{line}");
        }
        for line in [
            "/threadsx",
            "/merged",
            "hello",
            "/threads are slow",
            "/merge requests failing on CI",
            "/pull a b",
            "/park a b",
        ] {
            assert!(!is_cell_command(line), "{line}");
        }
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
