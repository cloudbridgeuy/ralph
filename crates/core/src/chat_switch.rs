use crate::chat_hold::{command_args, word_completion, HoldCompletion};
use crate::chat_thread::{open_check, ThreadMeta, ThreadName};
use std::ffi::OsString;
use std::path::PathBuf;

const SWITCH: &str = "switch";
const SUBCOMMANDS: [&str; 6] = ["ls", "main", "merge", "park", "pull", SWITCH];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchTarget {
    Main,
    Thread(ThreadName),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchArgv {
    pub store: PathBuf,
    pub recent_history_tokens: Option<usize>,
    pub wrap_width: Option<usize>,
    pub skills: Vec<PathBuf>,
    pub permission_mode: Option<String>,
    pub worktree_dir: Option<String>,
    pub no_inbox_sound: bool,
    pub no_inbox_notification: bool,
    pub skip_summary: bool,
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

pub fn switch_blockers(workers: &[String], watches: &[String]) -> Result<(), String> {
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
    let targets = match args.trim_start().split_once(char::is_whitespace) {
        None => SUBCOMMANDS
            .iter()
            .filter(|word| current.is_some() || **word != "main")
            .map(|word| word.to_string())
            .collect(),
        Some((SWITCH, rest)) if !rest.trim_start().contains(char::is_whitespace) => threads
            .iter()
            .filter(|name| current.is_none_or(|now| now.as_str() != name.as_str()))
            .cloned()
            .collect(),
        Some(_) => return None,
    };
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
        assert_eq!(switch_blockers(&[], &[]), Ok(()));
    }

    #[test]
    fn workers_and_watches_are_named() {
        let workers = vec!["w1".to_string(), "w2".to_string()];
        let watches = vec!["k1".to_string()];
        assert_eq!(
            switch_blockers(&workers, &watches),
            Err("workers are running (w1, w2); close them before you switch.".to_string())
        );
        assert_eq!(
            switch_blockers(&[], &watches),
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
    fn switch_completion_lists_open_threads_without_main_or_the_current_one() {
        assert_eq!(
            complete("/thread switch ", Some("side")),
            Some(vec!["auth".into(), "sub".into()])
        );
        assert_eq!(
            complete("/thread switch s", None),
            Some(vec!["side".into(), "sub".into()])
        );
    }

    #[test]
    fn subcommand_completion_offers_main_only_inside_a_thread() {
        assert_eq!(
            complete("/thread ", Some("side")),
            Some(
                ["ls", "main", "merge", "park", "pull", "switch"]
                    .map(String::from)
                    .to_vec()
            )
        );
        assert_eq!(complete("/thread m", None), Some(vec!["merge".into()]));
    }

    #[test]
    fn completion_stops_after_the_name_and_after_other_subcommands() {
        assert_eq!(complete("/thread switch auth ", None), None);
        assert_eq!(complete("/thread park ", None), None);
        assert_eq!(complete("/thread", None), None);
        assert_eq!(complete("/main ", None), None);
        assert_eq!(complete("/fork a", None), None);
    }

    #[test]
    fn thread_completion_span_covers_the_partial_word() {
        let threads = ["auth".to_string()];
        let found = switch_completions("/thread switch au", 17, None, &threads).unwrap();
        assert_eq!(found.span, 15..17);
    }
}
