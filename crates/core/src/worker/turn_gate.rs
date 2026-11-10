use super::Command;
use crate::chat_memory::Trigger;
use crate::chat_send_it::Action;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub text: String,
    pub stops_run: bool,
}

pub fn command_allowed(
    command: &Command,
    pending: Option<Trigger>,
    run: Option<&[Action]>,
) -> Result<(), Refused> {
    let in_run = |command: &Command| {
        run.unwrap_or_default()
            .iter()
            .any(|action| permits(action, command))
    };
    match pending {
        Some(Trigger::User) => Ok(()),
        Some(Trigger::Worker) if matches!(command, Command::Reply { .. }) => {
            if run.is_none() || in_run(command) {
                Ok(())
            } else {
                Err(run_refusal(command))
            }
        }
        Some(Trigger::SendIt) if in_run(command) => Ok(()),
        Some(Trigger::SendIt) => Err(run_refusal(command)),
        _ => Err(Refused {
            text:
                "Only the person can direct workers. Relay this to them and wait for their answer."
                    .to_string(),
            stops_run: false,
        }),
    }
}

fn run_refusal(command: &Command) -> Refused {
    Refused {
        text: format!(
            "The approved run does not allow `{}`. The run stops; ralph tells the person.",
            refused_tool(command)
        ),
        stops_run: true,
    }
}

fn permits(action: &Action, command: &Command) -> bool {
    match (action, command) {
        (Action::Spawn { harness }, Command::Spawn { harness: asked, .. }) => harness == asked,
        (Action::Reply, Command::Reply { .. }) | (Action::Watch, Command::Watch(_)) => true,
        (Action::Close, Command::Close { abandon, force, .. }) => !abandon && !force,
        _ => false,
    }
}

pub fn refused_tool(command: &Command) -> String {
    match command {
        Command::Spawn { harness, .. } => format!("spawn_worker {}", harness.as_str()),
        Command::Reply { .. } => "reply".to_string(),
        Command::Close { abandon, force, .. } => [
            Some("close_worker"),
            abandon.then_some("abandon"),
            force.then_some("force"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" "),
        Command::Watch(_) => "watch".to_string(),
        Command::Schedule(_) => "schedule".to_string(),
    }
}

pub const PENDING_TURN_FILE: &str = "turn";

pub fn pending_turn_marker(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::User => "user",
        Trigger::SendIt => "send_it",
        Trigger::Worker => "worker",
    }
}

pub fn parse_pending_turn(marker: &str) -> Option<Trigger> {
    match marker.trim() {
        "user" => Some(Trigger::User),
        "worker" => Some(Trigger::Worker),
        "send_it" => Some(Trigger::SendIt),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{parse_command, HarnessId};
    use serde_json::json;

    fn close() -> Command {
        close_with(false, false)
    }

    fn force_close() -> Command {
        close_with(true, true)
    }

    fn spawn() -> Command {
        spawn_on(HarnessId::ClaudeTmux)
    }

    fn reply() -> Command {
        Command::Reply {
            id: "A".into(),
            text: "t".into(),
            interrupt: false,
            request: None,
        }
    }

    fn parsed(name: &str, args: serde_json::Value) -> Command {
        match parse_command(name, &args) {
            Some(Ok(command)) => command,
            other => panic!("expected {name} to parse, got {other:?}"),
        }
    }

    fn watch() -> Command {
        parsed(
            "watch",
            json!({"id": "ci", "command": "c", "follow_up": "f"}),
        )
    }

    fn schedule() -> Command {
        parsed(
            "schedule",
            json!({"id": "a", "command": "c", "after": "5m"}),
        )
    }

    fn close_with(abandon: bool, force: bool) -> Command {
        Command::Close {
            id: "A".into(),
            abandon,
            force,
        }
    }

    fn spawn_on(harness: HarnessId) -> Command {
        Command::Spawn {
            id: "A".into(),
            task: "t".into(),
            harness,
            model: None,
            ticket: None,
            phase: None,
        }
    }

    fn every_action() -> Vec<Action> {
        vec![
            Action::Spawn {
                harness: HarnessId::ClaudeTmux,
            },
            Action::Reply,
            Action::Close,
            Action::Watch,
            Action::Merge,
            Action::Push,
            Action::Pr,
            Action::LinearWrite,
            Action::PaidRun { max_usd: 1.0 },
        ]
    }

    fn every_command() -> [Command; 6] {
        [
            close(),
            force_close(),
            spawn(),
            reply(),
            watch(),
            schedule(),
        ]
    }

    #[test]
    fn user_turn_allows_every_command_in_or_out_of_a_run() {
        for command in every_command() {
            for run in [None, Some(&[][..])] {
                assert!(command_allowed(&command, Some(Trigger::User), run).is_ok());
            }
        }
    }

    #[test]
    fn worker_turn_outside_a_run_allows_only_reply() {
        assert!(command_allowed(&reply(), Some(Trigger::Worker), None).is_ok());
        let every = every_action();
        for command in [close(), force_close(), spawn(), watch(), schedule()] {
            for run in [None, Some(every.as_slice())] {
                let refused = command_allowed(&command, Some(Trigger::Worker), run).unwrap_err();
                assert!(!refused.stops_run, "{command:?} with {run:?}");
                assert!(refused.text.starts_with("Only the person"));
            }
        }
    }

    #[test]
    fn worker_turn_in_a_run_refuses_a_reply_the_plan_does_not_list() {
        let refused = command_allowed(&reply(), Some(Trigger::Worker), Some(&[Action::Watch][..]))
            .unwrap_err();
        assert!(refused.stops_run);
        assert_eq!(
            refused.text,
            "The approved run does not allow `reply`. The run stops; ralph tells the person."
        );
    }

    #[test]
    fn worker_turn_in_a_run_allows_a_reply_the_plan_lists() {
        assert!(
            command_allowed(&reply(), Some(Trigger::Worker), Some(&[Action::Reply][..])).is_ok()
        );
    }

    #[test]
    fn no_pending_turn_allows_nothing_and_never_stops_a_run() {
        for command in every_command() {
            let refused =
                command_allowed(&command, None, Some(every_action().as_slice())).unwrap_err();
            assert!(!refused.stops_run);
        }
    }

    #[test]
    fn send_it_turn_allows_only_what_the_run_lists() {
        let claude = Action::Spawn {
            harness: HarnessId::ClaudeTmux,
        };
        let cases = [
            (spawn_on(HarnessId::ClaudeTmux), vec![claude.clone()], true),
            (spawn_on(HarnessId::Codex), vec![claude.clone()], false),
            (spawn_on(HarnessId::ClaudeTmux), vec![Action::Reply], false),
            (reply(), vec![Action::Reply], true),
            (reply(), vec![], false),
            (close(), vec![Action::Close], true),
            (close(), vec![Action::Reply], false),
            (close_with(true, false), vec![Action::Close], false),
            (close_with(false, true), vec![Action::Close], false),
            (force_close(), every_action(), false),
            (watch(), vec![Action::Watch], true),
            (watch(), vec![Action::Close], false),
            (schedule(), every_action(), false),
        ];
        for (command, allowed, expected) in cases {
            let gate = command_allowed(&command, Some(Trigger::SendIt), Some(allowed.as_slice()));
            assert_eq!(gate.is_ok(), expected, "{command:?} with {allowed:?}");
            assert!(gate.map_or_else(|refused| refused.stops_run, |()| true));
        }
    }

    #[test]
    fn actions_without_a_tool_unlock_nothing_in_a_run() {
        let toolless = [
            Action::Merge,
            Action::Push,
            Action::Pr,
            Action::LinearWrite,
            Action::PaidRun { max_usd: 1.0 },
        ];
        for command in every_command() {
            assert!(command_allowed(&command, Some(Trigger::SendIt), Some(&toolless[..])).is_err());
        }
    }

    #[test]
    fn a_run_refusal_names_the_tool_and_says_the_run_stops() {
        let refused = command_allowed(
            &spawn_on(HarnessId::Codex),
            Some(Trigger::SendIt),
            Some(&[][..]),
        )
        .unwrap_err();
        assert!(refused.stops_run);
        assert_eq!(
            refused.text,
            "The approved run does not allow `spawn_worker codex`. The run stops; ralph tells the person."
        );
    }

    #[test]
    fn refused_tool_names_the_tool_and_its_risky_options() {
        for (command, name) in [
            (spawn_on(HarnessId::Codex), "spawn_worker codex"),
            (spawn_on(HarnessId::Claude), "spawn_worker claude"),
            (reply(), "reply"),
            (close(), "close_worker"),
            (close_with(true, false), "close_worker abandon"),
            (force_close(), "close_worker abandon force"),
            (watch(), "watch"),
            (schedule(), "schedule"),
        ] {
            assert_eq!(refused_tool(&command), name);
        }
    }

    #[test]
    fn pending_turn_marker_round_trips() {
        for trigger in [Trigger::User, Trigger::Worker, Trigger::SendIt] {
            assert_eq!(
                parse_pending_turn(pending_turn_marker(trigger)),
                Some(trigger)
            );
        }
        assert_eq!(parse_pending_turn("junk"), None);
        assert_eq!(parse_pending_turn(""), None);
    }

    #[test]
    fn send_it_marker_is_distinct_from_user() {
        assert_ne!(
            pending_turn_marker(Trigger::SendIt),
            pending_turn_marker(Trigger::User)
        );
    }
}
