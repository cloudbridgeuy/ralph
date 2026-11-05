use crate::chat_help::Command;
use crate::chat_thread::ThreadName;

const THREADS_USAGE: &str = "usage: /threads takes no arguments and lists the threads.";
const PULL_USAGE: &str =
    "usage: /pull <name> moves thread <name> to main's end; run it in main's chat.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOp {
    Ls,
    Pull(ThreadName),
}

pub fn parse_thread_op(command: Command, args: &str) -> Result<ThreadOp, String> {
    match command {
        Command::Threads if args.trim().is_empty() => Ok(ThreadOp::Ls),
        Command::Threads => Err(THREADS_USAGE.to_string()),
        Command::Pull => thread_arg(PULL_USAGE, args).map(ThreadOp::Pull),
        other => Err(format!("{} is not a thread command.", other.name())),
    }
}

pub fn op_refusal(in_thread: bool, op: &ThreadOp) -> Option<String> {
    match op {
        ThreadOp::Ls => None,
        ThreadOp::Pull(_) => in_thread.then(|| main_only(Command::Pull)),
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
