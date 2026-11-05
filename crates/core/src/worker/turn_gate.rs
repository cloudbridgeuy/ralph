use super::Command;
use crate::chat_memory::Trigger;
use crate::chat_send_it::Action;

pub fn command_allowed(
    command: &Command,
    pending: Option<Trigger>,
    allowed: &[Action],
) -> Result<(), String> {
    match pending {
        Some(Trigger::User) => Ok(()),
        Some(Trigger::Worker) if matches!(command, Command::Reply { .. }) => Ok(()),
        Some(Trigger::SendIt) if allowed.iter().any(|action| permits(action, command)) => Ok(()),
        Some(Trigger::SendIt) => Err(format!(
            "The approved run does not allow `{}`. The run stops; ralph tells the person.",
            refused_tool(command)
        )),
        _ => Err(
            "Only the person can direct workers. Relay this to them and wait for their answer."
                .to_string(),
        ),
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
        spawn_on(HarnessId::Claude)
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
                harness: HarnessId::Claude,
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
    fn user_turn_allows_every_command_whatever_the_list() {
        for command in every_command() {
            assert!(command_allowed(&command, Some(Trigger::User), &[]).is_ok());
        }
    }

    #[test]
    fn worker_turn_allows_only_reply_whatever_the_list() {
        assert!(command_allowed(&reply(), Some(Trigger::Worker), &[]).is_ok());
        for command in [close(), force_close(), spawn(), watch(), schedule()] {
            assert!(command_allowed(&command, Some(Trigger::Worker), &every_action()).is_err());
        }
    }

    #[test]
    fn no_pending_turn_allows_nothing_whatever_the_list() {
        for command in every_command() {
            assert!(command_allowed(&command, None, &every_action()).is_err());
        }
    }

    #[test]
    fn send_it_turn_allows_only_what_the_run_lists() {
        let claude = Action::Spawn {
            harness: HarnessId::Claude,
        };
        let cases = [
            (spawn_on(HarnessId::Claude), vec![claude.clone()], true),
            (spawn_on(HarnessId::Codex), vec![claude.clone()], false),
            (spawn_on(HarnessId::Claude), vec![Action::Reply], false),
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
            assert_eq!(
                command_allowed(&command, Some(Trigger::SendIt), &allowed).is_ok(),
                expected,
                "{command:?} with {allowed:?}"
            );
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
            assert!(command_allowed(&command, Some(Trigger::SendIt), &toolless).is_err());
        }
    }

    #[test]
    fn a_run_refusal_names_the_tool_and_says_the_run_stops() {
        let refusal =
            command_allowed(&spawn_on(HarnessId::Codex), Some(Trigger::SendIt), &[]).unwrap_err();
        assert_eq!(
            refusal,
            "The approved run does not allow `spawn_worker codex`. The run stops; ralph tells the person."
        );
    }

    #[test]
    fn refused_tool_names_the_tool_and_its_risky_options() {
        for (command, name) in [
            (spawn_on(HarnessId::Codex), "spawn_worker codex"),
            (
                spawn_on(HarnessId::ClaudeHeadless),
                "spawn_worker claude-headless",
            ),
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
