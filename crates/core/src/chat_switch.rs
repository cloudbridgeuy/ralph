use crate::chat_hold::{command_args, word_completion, HoldCompletion};
use crate::chat_thread::{open_check, ThreadMeta, ThreadName};
use std::ffi::OsString;
use std::path::PathBuf;

const MAIN: &str = "main";
const THREAD_USAGE: &str =
    "usage: /thread <name> switches this chat to thread <name>; /thread main returns to main.";
const MAIN_USAGE: &str = "usage: /main takes no arguments and switches this chat to main.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchTarget {
    Main,
    Thread(ThreadName),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchArgv {
    pub store: PathBuf,
    pub name: Option<String>,
    pub recent_history_tokens: Option<usize>,
    pub wrap_width: Option<usize>,
    pub skills: Vec<PathBuf>,
    pub permission_mode: Option<String>,
    pub worktree_dir: Option<String>,
    pub no_inbox_sound: bool,
    pub no_inbox_notification: bool,
    pub skip_summary: bool,
}

pub fn parse_switch_args(args: &str) -> Result<SwitchTarget, String> {
    let mut words = args.split_whitespace();
    match (words.next(), words.next()) {
        (Some(MAIN), None) => Ok(SwitchTarget::Main),
        (Some(word), None) => ThreadName::parse(word)
            .map(SwitchTarget::Thread)
            .map_err(|e| format!("{e}\n{THREAD_USAGE}")),
        _ => Err(THREAD_USAGE.to_string()),
    }
}

pub fn parse_main_args(args: &str) -> Result<SwitchTarget, String> {
    match args.split_whitespace().next() {
        None => Ok(SwitchTarget::Main),
        Some(_) => Err(MAIN_USAGE.to_string()),
    }
}

pub fn switch_decision(
    current: Option<&ThreadName>,
    target: &SwitchTarget,
    meta: Option<&ThreadMeta>,
) -> Result<(), String> {
    match (current, target) {
        (None, SwitchTarget::Main) => Err("already in main.".to_string()),
        (Some(now), SwitchTarget::Thread(next)) if now == next => {
            Err(format!("already in thread {now}."))
        }
        (_, SwitchTarget::Thread(next)) => meta.map_or(Ok(()), |meta| open_check(next, meta)),
        (Some(_), SwitchTarget::Main) => Ok(()),
    }
}

pub fn switch_blockers(
    workers: &[String],
    watches: &[String],
    turn_running: bool,
) -> Result<(), String> {
    if turn_running {
        return Err("a turn is running; wait for it to end before you switch.".to_string());
    }
    if !workers.is_empty() {
        return Err(format!(
            "workers are running ({}); close them before you switch.",
            workers.join(", ")
        ));
    }
    if !watches.is_empty() {
        return Err(format!(
            "watches are running ({}); stop them before you switch.",
            watches.join(", ")
        ));
    }
    Ok(())
}

pub fn switch_argv(opts: &SwitchArgv, target: &SwitchTarget) -> Vec<OsString> {
    let mut argv: Vec<OsString> = vec!["chat".into(), "--store".into(), opts.store.clone().into()];
    if let SwitchTarget::Thread(name) = target {
        argv.extend(["--thread".into(), name.to_string().into()]);
    }
    if let Some(name) = &opts.name {
        argv.extend(["--name".into(), name.into()]);
    }
    if let Some(tokens) = opts.recent_history_tokens {
        argv.extend(["--recent-history-tokens".into(), tokens.to_string().into()]);
    }
    if let Some(width) = opts.wrap_width {
        argv.extend(["--wrap-width".into(), width.to_string().into()]);
    }
    for skill in &opts.skills {
        argv.extend(["--skill".into(), skill.clone().into()]);
    }
    if let Some(mode) = &opts.permission_mode {
        argv.extend(["--permission-mode".into(), mode.into()]);
    }
    if let Some(dir) = &opts.worktree_dir {
        argv.extend(["--worktree-dir".into(), dir.into()]);
    }
    let flags = [
        (opts.no_inbox_sound, "--no-inbox-sound"),
        (opts.no_inbox_notification, "--no-inbox-notification"),
        (opts.skip_summary, "--skip-summary"),
    ];
    argv.extend(
        flags
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, flag)| flag.into()),
    );
    argv
}

pub fn switch_completions(
    line: &str,
    cursor: usize,
    current: Option<&ThreadName>,
    threads: &[String],
) -> Option<HoldCompletion> {
    let args = command_args(line, cursor, "/thread")?;
    if args.trim_start().contains(char::is_whitespace) {
        return None;
    }
    let main = current.map(|_| MAIN.to_string());
    let targets = threads
        .iter()
        .filter(|name| current.is_none_or(|now| now.as_str() != name.as_str()))
        .cloned()
        .chain(main)
        .collect();
    Some(word_completion(args, cursor, targets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_thread::{Merged, Parked, ThreadMeta, ThreadName};
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn name(s: &str) -> ThreadName {
        ThreadName::parse(s).unwrap()
    }

    fn meta() -> ThreadMeta {
        ThreadMeta {
            base: 3,
            created: "now".to_string(),
            merged: None,
            parked: None,
        }
    }

    fn argv_opts() -> SwitchArgv {
        SwitchArgv {
            store: PathBuf::from("/main/store"),
            name: None,
            recent_history_tokens: None,
            wrap_width: None,
            skills: Vec::new(),
            permission_mode: None,
            worktree_dir: None,
            no_inbox_sound: false,
            no_inbox_notification: false,
            skip_summary: false,
        }
    }

    fn strings(argv: Vec<OsString>) -> Vec<String> {
        argv.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn thread_args_parse_one_name() {
        assert_eq!(
            parse_switch_args("  side "),
            Ok(SwitchTarget::Thread(name("side")))
        );
    }

    #[test]
    fn the_word_main_means_main() {
        assert_eq!(parse_switch_args("main"), Ok(SwitchTarget::Main));
    }

    #[test]
    fn thread_args_refuse_no_name_or_two_names() {
        let usage = "usage: /thread <name> switches this chat to thread <name>; /thread main returns to main.";
        assert_eq!(parse_switch_args(""), Err(usage.to_string()));
        assert_eq!(parse_switch_args("a b"), Err(usage.to_string()));
    }

    #[test]
    fn thread_args_refuse_a_bad_name_with_usage() {
        let err = parse_switch_args("Bad_Name").unwrap_err();
        assert!(err.starts_with("'Bad_Name' is not a thread name."));
        assert!(err.ends_with("returns to main."));
    }

    #[test]
    fn main_args_take_no_words() {
        assert_eq!(parse_main_args("  "), Ok(SwitchTarget::Main));
        assert_eq!(
            parse_main_args("side"),
            Err("usage: /main takes no arguments and switches this chat to main.".to_string())
        );
    }

    #[test]
    fn same_target_is_refused() {
        assert_eq!(
            switch_decision(None, &SwitchTarget::Main, None),
            Err("already in main.".to_string())
        );
        let side = name("side");
        assert_eq!(
            switch_decision(
                Some(&side),
                &SwitchTarget::Thread(side.clone()),
                Some(&meta())
            ),
            Err("already in thread side.".to_string())
        );
    }

    #[test]
    fn other_targets_are_allowed() {
        let side = name("side");
        assert_eq!(
            switch_decision(Some(&side), &SwitchTarget::Main, None),
            Ok(())
        );
        assert_eq!(
            switch_decision(None, &SwitchTarget::Thread(side.clone()), Some(&meta())),
            Ok(())
        );
        assert_eq!(
            switch_decision(Some(&name("a")), &SwitchTarget::Thread(side), Some(&meta())),
            Ok(())
        );
    }

    #[test]
    fn an_unknown_thread_is_allowed() {
        assert_eq!(
            switch_decision(None, &SwitchTarget::Thread(name("new")), None),
            Ok(())
        );
    }

    #[test]
    fn merged_and_parked_threads_are_refused() {
        let side = name("side");
        let merged = ThreadMeta {
            merged: Some(Merged {
                at: "t".to_string(),
                digest: "d".to_string(),
            }),
            ..meta()
        };
        let parked = ThreadMeta {
            parked: Some(Parked {
                at: "t".to_string(),
            }),
            ..meta()
        };
        let target = SwitchTarget::Thread(side.clone());
        assert_eq!(
            switch_decision(None, &target, Some(&merged)),
            crate::chat_thread::open_check(&side, &merged)
        );
        assert!(switch_decision(None, &target, Some(&merged))
            .unwrap_err()
            .contains("merged"));
        assert!(switch_decision(None, &target, Some(&parked))
            .unwrap_err()
            .contains("parked"));
    }

    #[test]
    fn nothing_running_allows_the_switch() {
        assert_eq!(switch_blockers(&[], &[], false), Ok(()));
    }

    #[test]
    fn a_running_turn_blocks_first() {
        let workers = vec!["w1".to_string()];
        assert_eq!(
            switch_blockers(&workers, &[], true),
            Err("a turn is running; wait for it to end before you switch.".to_string())
        );
    }

    #[test]
    fn workers_and_watches_are_named() {
        let workers = vec!["w1".to_string(), "w2".to_string()];
        let watches = vec!["k1".to_string()];
        assert_eq!(
            switch_blockers(&workers, &watches, false),
            Err("workers are running (w1, w2); close them before you switch.".to_string())
        );
        assert_eq!(
            switch_blockers(&[], &watches, false),
            Err("watches are running (k1); stop them before you switch.".to_string())
        );
    }

    #[test]
    fn main_argv_has_only_the_store() {
        assert_eq!(
            strings(switch_argv(&argv_opts(), &SwitchTarget::Main)),
            ["chat", "--store", "/main/store"]
        );
    }

    #[test]
    fn thread_argv_adds_the_thread_after_the_store() {
        assert_eq!(
            strings(switch_argv(
                &argv_opts(),
                &SwitchTarget::Thread(name("side"))
            )),
            ["chat", "--store", "/main/store", "--thread", "side"]
        );
    }

    #[test]
    fn every_option_is_forwarded() {
        let opts = SwitchArgv {
            name: Some("me".to_string()),
            recent_history_tokens: Some(500),
            wrap_width: Some(80),
            skills: vec![PathBuf::from("/s1"), PathBuf::from("/s2")],
            permission_mode: Some("plan".to_string()),
            worktree_dir: Some("/wt".to_string()),
            no_inbox_sound: true,
            no_inbox_notification: true,
            skip_summary: true,
            ..argv_opts()
        };
        assert_eq!(
            strings(switch_argv(&opts, &SwitchTarget::Main)),
            [
                "chat",
                "--store",
                "/main/store",
                "--name",
                "me",
                "--recent-history-tokens",
                "500",
                "--wrap-width",
                "80",
                "--skill",
                "/s1",
                "--skill",
                "/s2",
                "--permission-mode",
                "plan",
                "--worktree-dir",
                "/wt",
                "--no-inbox-sound",
                "--no-inbox-notification",
                "--skip-summary",
            ]
        );
    }

    fn complete(line: &str, current: Option<&str>) -> Option<Vec<String>> {
        let current = current.map(name);
        let threads = ["auth", "side", "sub"].map(String::from);
        switch_completions(line, line.len(), current.as_ref(), &threads).map(|c| c.values)
    }

    #[test]
    fn thread_completion_lists_open_threads_and_main() {
        assert_eq!(
            complete("/thread ", Some("side")),
            Some(vec!["auth".into(), "main".into(), "sub".into()])
        );
    }

    #[test]
    fn thread_completion_in_main_has_no_main_entry() {
        assert_eq!(
            complete("/thread s", None),
            Some(vec!["side".into(), "sub".into()])
        );
    }

    #[test]
    fn thread_completion_takes_one_word_and_only_the_thread_command() {
        assert_eq!(complete("/thread auth ", None), None);
        assert_eq!(complete("/thread", None), None);
        assert_eq!(complete("/main ", None), None);
        assert_eq!(complete("/fork a", None), None);
    }

    #[test]
    fn thread_completion_span_covers_the_partial_word() {
        let threads = ["auth".to_string()];
        let found = switch_completions("/thread au", 10, None, &threads).unwrap();
        assert_eq!(found.span, 8..10);
    }
}
