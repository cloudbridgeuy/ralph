use crate::chat_help::Command;

const THREADS_USAGE: &str = "usage: /threads takes no arguments and lists the threads.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOp {
    Ls,
}

pub fn parse_thread_op(command: Command, args: &str) -> Result<ThreadOp, String> {
    match command {
        Command::Threads if args.trim().is_empty() => Ok(ThreadOp::Ls),
        Command::Threads => Err(THREADS_USAGE.to_string()),
        other => Err(format!("{} is not a thread command.", other.name())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_help::Command;

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
