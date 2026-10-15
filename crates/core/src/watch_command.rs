use crate::watch::{watch_id, WatchKind, WatchSpec};
use crate::watch_request::WatchRequest;
use crate::watch_store::SavedWatch;
use chrono::{DateTime, Utc};
use std::time::Duration;

pub const WATCH_USAGE: &str =
    "Usage: /watch start [--persistent] <command> -- <follow-up> | list | stop <id>";
const PERSISTENT_FLAG: &str = "--persistent";
const FOLLOW_UP_SEPARATOR: &str = " -- ";
const NO_TIME_LEFT: &str = "—";
const NO_WATCHES: &str = "No active watches.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartWatch {
    pub persistent: bool,
    pub command: String,
    pub follow_up: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchCommand {
    Start(StartWatch),
    List,
    Stop(String),
    Usage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopResult {
    Stopped,
    RemovedSaved,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopAction {
    KillRunning,
    RemoveSaved,
    Unknown,
}

impl StopAction {
    pub fn result(self) -> StopResult {
        match self {
            Self::KillRunning => StopResult::Stopped,
            Self::RemoveSaved => StopResult::RemovedSaved,
            Self::Unknown => StopResult::Unknown,
        }
    }
}

pub fn decide_stop(id: &str, running: &[String], saved: &[String]) -> StopAction {
    if running.iter().any(|running| running == id) {
        StopAction::KillRunning
    } else if saved.iter().any(|saved| saved == id) {
        StopAction::RemoveSaved
    } else {
        StopAction::Unknown
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartPlan {
    pub spec: WatchSpec,
    pub confirmation: String,
}

pub fn plan_start(start: StartWatch, used: &[String]) -> StartPlan {
    let spec = WatchRequest {
        id: next_watch_id(used),
        command: start.command,
        follow_up: start.follow_up,
        persistent: start.persistent,
        timeout_secs: None,
    }
    .into_spec();
    let confirmation = start_message(&spec.id, spec.kind);
    StartPlan { spec, confirmation }
}

pub fn parse_watch_args(args: &str) -> WatchCommand {
    let args = args.trim();
    let (word, rest) = args
        .split_once(char::is_whitespace)
        .map_or((args, ""), |(word, rest)| (word, rest.trim()));
    match word {
        "start" => parse_start(rest),
        "list" if rest.is_empty() => WatchCommand::List,
        "stop" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
            [id] => WatchCommand::Stop(watch_id(id)),
            _ => WatchCommand::Usage,
        },
        _ => WatchCommand::Usage,
    }
}

fn parse_start(rest: &str) -> WatchCommand {
    let (persistent, body) = match rest.strip_prefix(PERSISTENT_FLAG) {
        Some(after) if after.starts_with(char::is_whitespace) => (true, after.trim_start()),
        _ => (false, rest),
    };
    let Some((command, follow_up)) = body.rsplit_once(FOLLOW_UP_SEPARATOR) else {
        return WatchCommand::Usage;
    };
    let (command, follow_up) = (command.trim(), follow_up.trim());
    if command.is_empty() || follow_up.is_empty() {
        return WatchCommand::Usage;
    }
    WatchCommand::Start(StartWatch {
        persistent,
        command: command.to_string(),
        follow_up: follow_up.to_string(),
    })
}

pub fn next_watch_id(used: &[String]) -> String {
    (1u64..)
        .map(|n| watch_id(&format!("w{n}")))
        .find(|id| !used.contains(id))
        .unwrap_or_default()
}

pub fn time_left(saved: &SavedWatch, now: DateTime<Utc>) -> Option<Duration> {
    [saved.wake_deadline, saved.timeout_deadline]
        .into_iter()
        .flatten()
        .min()
        .map(|deadline| (deadline - now).to_std().unwrap_or_default())
}

pub fn format_left(left: Duration) -> String {
    let secs = left.as_secs();
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

fn left_label(saved: &SavedWatch, now: DateTime<Utc>) -> String {
    time_left(saved, now).map_or_else(|| NO_TIME_LEFT.to_string(), format_left)
}

pub fn kind_label(kind: WatchKind) -> &'static str {
    match kind {
        WatchKind::Normal => "normal",
        WatchKind::Persistent => "persistent",
    }
}

pub fn list_watches(saved: &[SavedWatch], now: DateTime<Utc>) -> String {
    if saved.is_empty() {
        return NO_WATCHES.to_string();
    }
    let lefts: Vec<String> = saved.iter().map(|watch| left_label(watch, now)).collect();
    let id_width = saved
        .iter()
        .map(|w| w.id.chars().count())
        .max()
        .unwrap_or(0);
    let left_width = lefts.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    saved
        .iter()
        .zip(&lefts)
        .map(|(watch, left)| {
            format!(
                "{:<id_width$}  {:<10}  {:>left_width$}  {}",
                watch.id,
                kind_label(watch.kind),
                left,
                watch.command
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn start_message(id: &str, kind: WatchKind) -> String {
    format!("Started {} watch {id}.", kind_label(kind))
}

pub fn stop_message(id: &str, result: StopResult) -> String {
    match result {
        StopResult::Stopped => format!("Stopped watch {id}."),
        StopResult::RemovedSaved => {
            format!("Watch {id} was not running. Removed its saved record.")
        }
        StopResult::Unknown => format!("No watch {id}. /watch list shows active watches."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(persistent: bool, command: &str, follow_up: &str) -> WatchCommand {
        WatchCommand::Start(StartWatch {
            persistent,
            command: command.to_string(),
            follow_up: follow_up.to_string(),
        })
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn saved(id: &str, command: &str, kind: WatchKind, timeout: Option<u64>) -> SavedWatch {
        SavedWatch::new(
            &WatchSpec {
                id: id.to_string(),
                command: command.to_string(),
                follow_up: Some("f".to_string()),
                kind,
                timeout: timeout.map(Duration::from_secs),
                limit: timeout.map(Duration::from_secs),
            },
            at(0),
        )
    }

    #[test]
    fn parses_start_with_and_without_the_flag() {
        assert_eq!(
            parse_watch_args("start make test -- tell me"),
            start(false, "make test", "tell me")
        );
        assert_eq!(
            parse_watch_args("  start   --persistent  sleep 60 --  ping me "),
            start(true, "sleep 60", "ping me")
        );
    }

    #[test]
    fn splits_at_the_last_separator() {
        assert_eq!(
            parse_watch_args("start a -- b -- c"),
            start(false, "a -- b", "c")
        );
    }

    #[test]
    fn rejects_start_without_both_parts() {
        for args in [
            "start",
            "start cmd",
            "start cmd --",
            "start cmd -- ",
            "start -- follow",
            "start --persistent",
            "start --persistent -- follow",
            "start --persistentx cmd",
        ] {
            assert_eq!(parse_watch_args(args), WatchCommand::Usage, "{args}");
        }
    }

    #[test]
    fn parses_list_and_stop() {
        assert_eq!(parse_watch_args("list"), WatchCommand::List);
        assert_eq!(
            parse_watch_args(" stop w3 "),
            WatchCommand::Stop("watch:w3".into())
        );
        assert_eq!(
            parse_watch_args("stop watch:w3"),
            WatchCommand::Stop("watch:w3".into())
        );
    }

    #[test]
    fn bad_arguments_give_usage() {
        for args in ["", "list now", "stop", "stop a b", "nope", "peek 3"] {
            assert_eq!(parse_watch_args(args), WatchCommand::Usage, "{args}");
        }
    }

    #[test]
    fn stop_prefers_running_then_saved_then_unknown() {
        let ids = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        let running = ids(&["watch:a"]);
        let saved = ids(&["watch:a", "watch:b"]);
        assert_eq!(
            decide_stop("watch:a", &running, &saved),
            StopAction::KillRunning
        );
        assert_eq!(
            decide_stop("watch:b", &running, &saved),
            StopAction::RemoveSaved
        );
        assert_eq!(
            decide_stop("watch:c", &running, &saved),
            StopAction::Unknown
        );
    }

    #[test]
    fn stop_actions_map_to_results_and_messages() {
        assert_eq!(StopAction::KillRunning.result(), StopResult::Stopped);
        assert_eq!(StopAction::RemoveSaved.result(), StopResult::RemovedSaved);
        assert_eq!(StopAction::Unknown.result(), StopResult::Unknown);
        assert_eq!(
            stop_message("watch:a", StopAction::KillRunning.result()),
            "Stopped watch watch:a."
        );
    }

    #[test]
    fn plan_start_builds_a_spec_without_its_own_timeout_and_a_confirmation() {
        let plan = plan_start(
            StartWatch {
                persistent: true,
                command: "make".into(),
                follow_up: "report".into(),
            },
            &["watch:w1".to_string()],
        );
        assert_eq!(plan.spec.id, "watch:w2");
        assert_eq!(plan.spec.command, "make");
        assert_eq!(plan.spec.follow_up.as_deref(), Some("report"));
        assert_eq!(plan.spec.kind, WatchKind::Persistent);
        assert_eq!(plan.spec.timeout, None);
        assert_eq!(plan.confirmation, "Started persistent watch watch:w2.");
    }

    #[test]
    fn next_id_is_the_lowest_unused() {
        let ids = |list: &[&str]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(next_watch_id(&[]), "watch:w1");
        assert_eq!(next_watch_id(&ids(&["watch:w1", "watch:w2"])), "watch:w3");
        assert_eq!(
            next_watch_id(&ids(&["watch:w2", "watch:ci", "watch:w1", "watch:w4"])),
            "watch:w3"
        );
        assert_eq!(next_watch_id(&ids(&["w1", "watch:w1x"])), "watch:w1");
    }

    #[test]
    fn time_left_takes_the_earliest_deadline() {
        let both = saved("a", "sleep 100", WatchKind::Normal, Some(30));
        assert_eq!(time_left(&both, at(10)), Some(Duration::from_secs(20)));
        let sleep_only = saved("a", "sleep 100", WatchKind::Normal, None);
        assert_eq!(
            time_left(&sleep_only, at(10)),
            Some(Duration::from_secs(90))
        );
        let none = saved("a", "make", WatchKind::Normal, None);
        assert_eq!(time_left(&none, at(10)), None);
    }

    #[test]
    fn time_left_floors_at_zero() {
        let overdue = saved("a", "sleep 5", WatchKind::Normal, None);
        assert_eq!(time_left(&overdue, at(50)), Some(Duration::ZERO));
    }

    #[test]
    fn format_left_picks_the_largest_unit() {
        for (secs, label) in [
            (0, "0s"),
            (59, "59s"),
            (60, "1m"),
            (3599, "59m"),
            (3600, "1h"),
            (86399, "23h"),
            (86400, "1d"),
        ] {
            assert_eq!(format_left(Duration::from_secs(secs)), label);
        }
    }

    #[test]
    fn list_of_nothing_says_so() {
        assert_eq!(list_watches(&[], at(0)), "No active watches.");
    }

    #[test]
    fn list_shows_id_kind_left_and_command() {
        let watches = [
            saved("w1", "sleep 600", WatchKind::Persistent, None),
            saved("ci", "make test", WatchKind::Normal, None),
        ];
        assert_eq!(
            list_watches(&watches, at(0)),
            "w1  persistent  10m  sleep 600\nci  normal        —  make test"
        );
    }

    #[test]
    fn messages_name_the_watch() {
        assert_eq!(
            start_message("w1", WatchKind::Persistent),
            "Started persistent watch w1."
        );
        assert_eq!(stop_message("w1", StopResult::Stopped), "Stopped watch w1.");
        assert!(stop_message("w1", StopResult::RemovedSaved).contains("saved record"));
        assert!(stop_message("w9", StopResult::Unknown).starts_with("No watch w9."));
    }
}
