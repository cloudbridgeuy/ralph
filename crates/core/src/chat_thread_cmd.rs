use crate::chat_help::Command;
use crate::chat_thread::ThreadName;

pub const REBASE_FLAG: &str = "--rebase";
const THREADS_USAGE: &str = "usage: /threads takes no arguments and lists the threads.";
const PARK_USAGE: &str =
    "usage: /park <name> stops thread <name> from opening; run it in main's chat.";
const PULL_USAGE: &str =
    "usage: /pull <name> moves thread <name> to main's end; run it in main's chat.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOp {
    Ls,
    Pull(ThreadName),
    Park(ThreadName),
    Rebase(ThreadName),
}

pub fn parse_thread_op(command: Command, args: &str) -> Result<ThreadOp, String> {
    match command {
        Command::Threads if args.trim().is_empty() => Ok(ThreadOp::Ls),
        Command::Threads => Err(THREADS_USAGE.to_string()),
        Command::Pull => thread_arg(PULL_USAGE, args).map(ThreadOp::Pull),
        Command::Park => thread_arg(PARK_USAGE, args).map(ThreadOp::Park),
        Command::Merge => parse_merge(args),
        other => Err(format!("{} is not a thread command.", other.name())),
    }
}

pub fn op_refusal(in_thread: bool, op: &ThreadOp) -> Option<String> {
    match op {
        ThreadOp::Ls => None,
        ThreadOp::Pull(_) => in_thread.then(|| main_only(Command::Pull)),
        ThreadOp::Park(_) => in_thread.then(|| main_only(Command::Park)),
        ThreadOp::Rebase(_) => in_thread.then(|| main_only(Command::Merge)),
    }
}

fn merge_usage() -> String {
    format!(
        "usage: /merge <name> {REBASE_FLAG} posts thread <name>'s memory view to main's inbox with no model call; run it in main's chat."
    )
}

fn parse_merge(args: &str) -> Result<ThreadOp, String> {
    let (flags, rest): (Vec<&str>, Vec<&str>) = args
        .split_whitespace()
        .partition(|word| *word == REBASE_FLAG);
    match (flags.len(), rest.as_slice()) {
        (1, [word]) => ThreadName::parse(word)
            .map(ThreadOp::Rebase)
            .map_err(|e| format!("{e}\n{}", merge_usage())),
        _ => Err(merge_usage()),
    }
}

fn main_only(command: Command) -> String {
    let name = command.name();
    format!("{name} runs in main's chat. Run /main, then {name} <name>.")
}

fn thread_arg(usage: &str, args: &str) -> Result<ThreadName, String> {
    let mut words = args.split_whitespace();
    match (words.next(), words.next()) {
        (Some(word), None) => ThreadName::parse(word).map_err(|e| format!("{e}\n{usage}")),
        _ => Err(usage.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_help::Command;
    use crate::chat_thread::ThreadName;

    #[test]
    fn threads_without_arguments_lists() {
        assert_eq!(parse_thread_op(Command::Threads, ""), Ok(ThreadOp::Ls));
        assert_eq!(parse_thread_op(Command::Threads, "  "), Ok(ThreadOp::Ls));
    }

    #[test]
    fn threads_with_an_argument_is_a_usage_error() {
        let err = parse_thread_op(Command::Threads, " auth").unwrap_err();
        assert!(err.starts_with("usage: /threads"), "{err}");
    }

    #[test]
    fn a_command_that_is_not_a_thread_command_is_refused() {
        let err = parse_thread_op(Command::Help, "").unwrap_err();
        assert!(err.contains("/help"), "{err}");
    }

    fn auth() -> ThreadName {
        ThreadName::parse("auth").unwrap()
    }

    #[test]
    fn pull_takes_one_thread_name() {
        assert_eq!(
            parse_thread_op(Command::Pull, " auth "),
            Ok(ThreadOp::Pull(auth()))
        );
    }

    #[test]
    fn pull_without_a_name_or_with_extra_words_is_a_usage_error() {
        for args in ["", "auth now", "auth\nfix"] {
            let err = parse_thread_op(Command::Pull, args).unwrap_err();
            assert!(err.starts_with("usage: /pull <name>"), "{err}");
        }
    }

    #[test]
    fn pull_with_a_bad_name_names_the_problem_and_the_usage() {
        let err = parse_thread_op(Command::Pull, "Bad_Name").unwrap_err();
        assert!(err.contains("usage: /pull <name>"), "{err}");
        assert!(err.lines().count() > 1, "{err}");
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
            Ok(ThreadOp::Park(auth()))
        );
    }

    #[test]
    fn park_without_a_name_or_with_extra_words_is_a_usage_error() {
        for args in ["", "auth now"] {
            let err = parse_thread_op(Command::Park, args).unwrap_err();
            assert!(err.starts_with("usage: /park <name>"), "{err}");
        }
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

    #[test]
    fn merge_with_the_rebase_flag_in_either_place_rebases() {
        let want = Ok(ThreadOp::Rebase(auth()));
        let line = format!("auth {REBASE_FLAG}");
        assert_eq!(parse_thread_op(Command::Merge, &line), want);
        let line = format!("{REBASE_FLAG}  auth");
        assert_eq!(parse_thread_op(Command::Merge, &line), want);
    }

    #[test]
    fn merge_without_the_flag_or_a_name_or_with_extras_is_a_usage_error() {
        let both = format!("auth other {REBASE_FLAG}");
        let twice = format!("auth {REBASE_FLAG} {REBASE_FLAG}");
        let only = REBASE_FLAG.to_string();
        for args in ["auth", "", &only, &both, &twice] {
            let err = parse_thread_op(Command::Merge, args).unwrap_err();
            assert!(err.starts_with("usage: /merge <name> --rebase"), "{err}");
        }
    }

    #[test]
    fn merge_usage_and_hint_use_the_flag_constant() {
        assert_eq!(REBASE_FLAG, "--rebase");
        let err = parse_thread_op(Command::Merge, "").unwrap_err();
        assert!(err.contains(REBASE_FLAG), "{err}");
    }

    #[test]
    fn merge_with_a_bad_name_names_the_problem_and_the_usage() {
        let line = format!("Bad_Name {REBASE_FLAG}");
        let err = parse_thread_op(Command::Merge, &line).unwrap_err();
        assert!(err.contains("usage: /merge <name>"), "{err}");
        assert!(err.lines().count() > 1, "{err}");
    }

    #[test]
    fn a_thread_chat_refuses_rebase_with_a_hint_and_main_allows_it() {
        let rebase = ThreadOp::Rebase(auth());
        assert_eq!(
            op_refusal(true, &rebase),
            Some("/merge runs in main's chat. Run /main, then /merge <name>.".to_string())
        );
        assert_eq!(op_refusal(false, &rebase), None);
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
        assert!(matches!(
            route("/threads x"),
            Route::ThreadOp { op: Err(_), .. }
        ));
        assert!(matches!(route("/threadsx"), Route::Send(_)));
        assert!(matches!(route("/thread auth"), Route::Switch { .. }));
    }
}
