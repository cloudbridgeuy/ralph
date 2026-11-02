use super::Command;
use crate::chat_memory::Trigger;

pub fn command_allowed(command: &Command, pending: Option<Trigger>) -> Result<(), String> {
    match (command, pending) {
        (_, Some(Trigger::User))
        | (Command::Reply { .. }, Some(Trigger::Worker | Trigger::SendIt)) => Ok(()),
        _ => Err(
            "Only the person can direct workers. Relay this to them and wait for their answer."
                .to_string(),
        ),
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
        Command::Close {
            id: "A".into(),
            abandon: false,
            force: false,
        }
    }

    fn force_close() -> Command {
        Command::Close {
            id: "A".into(),
            abandon: true,
            force: true,
        }
    }

    fn spawn() -> Command {
        Command::Spawn {
            id: "A".into(),
            task: "t".into(),
            harness: HarnessId::Claude,
            model: None,
            ticket: None,
        }
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

    #[test]
    fn user_turn_allows_every_command() {
        for command in [
            close(),
            force_close(),
            spawn(),
            reply(),
            watch(),
            schedule(),
        ] {
            assert!(command_allowed(&command, Some(Trigger::User)).is_ok());
        }
    }

    #[test]
    fn worker_turn_allows_only_reply() {
        assert!(command_allowed(&reply(), Some(Trigger::Worker)).is_ok());
        for command in [close(), force_close(), spawn(), watch(), schedule()] {
            assert!(command_allowed(&command, Some(Trigger::Worker)).is_err());
        }
    }

    #[test]
    fn send_it_turn_allows_only_reply() {
        assert!(command_allowed(&reply(), Some(Trigger::SendIt)).is_ok());
        for command in [close(), force_close(), spawn(), watch(), schedule()] {
            assert!(command_allowed(&command, Some(Trigger::SendIt)).is_err());
        }
    }

    #[test]
    fn no_pending_turn_allows_nothing() {
        for command in [close(), spawn(), reply(), watch(), schedule()] {
            assert!(command_allowed(&command, None).is_err());
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
