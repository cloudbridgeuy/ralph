use crate::chat_help::Command;
use crate::chat_thread::ThreadName;

pub const REBASE_FLAG: &str = "--rebase";
const PARK_USAGE: &str =
    "usage: /park <name> stops thread <name> from opening; run it in main's chat.";
const PULL_USAGE: &str =
    "usage: /pull <name> moves thread <name> to main's end; run it in main's chat.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOp {
    Ls,
    Pull(ThreadName),
    Park(ThreadName),
    Merge { name: ThreadName, rebase: bool },
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
        "usage: /merge <name> [{REBASE_FLAG}] merges thread <name> into main's inbox; {REBASE_FLAG} posts its memory view with no model call and no confirmation; run it in main's chat."
    )
}

fn parse_merge(args: &str) -> Option<ThreadOp> {
    let (flags, rest): (Vec<&str>, Vec<&str>) = args
        .split_whitespace()
        .partition(|word| *word == REBASE_FLAG);
    match (flags.len(), rest.as_slice()) {
        (0 | 1, [word]) => ThreadName::parse(word).ok().map(|name| ThreadOp::Merge {
            name,
            rebase: !flags.is_empty(),
        }),
        _ => None,
    }
}

pub fn needs_confirm(op: &ThreadOp) -> bool {
    matches!(op, ThreadOp::Merge { rebase: false, .. })
}

pub fn merge_question(name: &ThreadName) -> String {
    format!(
        "Merge thread {name} into main? This makes one paid Haiku call, and up to 3 if the digest is refused. [y/n] "
    )
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

    fn merge(rebase: bool) -> ThreadOp {
        ThreadOp::Merge {
            name: auth(),
            rebase,
        }
    }

    #[test]
    fn merge_without_the_flag_squashes() {
        assert_eq!(
            parse_thread_op(Command::Merge, " auth "),
            Some(Ok(merge(false)))
        );
    }

    #[test]
    fn merge_with_the_rebase_flag_in_either_place_rebases() {
        let line = format!("auth {REBASE_FLAG}");
        assert_eq!(
            parse_thread_op(Command::Merge, &line),
            Some(Ok(merge(true)))
        );
        let line = format!("{REBASE_FLAG}  auth");
        assert_eq!(
            parse_thread_op(Command::Merge, &line),
            Some(Ok(merge(true)))
        );
    }

    #[test]
    fn merge_without_arguments_is_a_usage_error() {
        let err = parse_thread_op(Command::Merge, "").unwrap().unwrap_err();
        assert!(err.starts_with("usage: /merge <name> [--rebase]"), "{err}");
        assert!(err.contains(&format!("[{REBASE_FLAG}]")), "{err}");
    }

    #[test]
    fn merge_with_the_wrong_shape_is_not_a_thread_op() {
        let both = format!("auth other {REBASE_FLAG}");
        let twice = format!("auth {REBASE_FLAG} {REBASE_FLAG}");
        for args in [REBASE_FLAG, "auth other", "Bad_Name", &both, &twice] {
            assert_eq!(parse_thread_op(Command::Merge, args), None, "{args}");
        }
    }

    #[test]
    fn a_thread_chat_refuses_both_merges_with_a_hint_and_main_allows_them() {
        for rebase in [false, true] {
            assert_eq!(
                op_refusal(true, &merge(rebase)),
                Some("/merge runs in main's chat. Run /main, then /merge <name>.".to_string())
            );
            assert_eq!(op_refusal(false, &merge(rebase)), None);
        }
    }

    #[test]
    fn only_the_squash_merge_needs_a_confirmation() {
        assert!(needs_confirm(&merge(false)));
        assert!(!needs_confirm(&merge(true)));
        for op in [ThreadOp::Ls, ThreadOp::Pull(auth()), ThreadOp::Park(auth())] {
            assert!(!needs_confirm(&op), "{op:?}");
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
            "/merge --rebase",
            "/merge auth --force",
            "/merge auth --rebase --rebase",
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
            route("/merge auth --rebase"),
            Route::ThreadOp { op: Ok(_), .. }
        ));
    }
}
