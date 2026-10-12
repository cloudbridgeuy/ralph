pub struct InputWatch {
    pub hangup_signal: bool,
    pub started_on_terminal: bool,
    pub stdin_hung_up: bool,
    pub at_prompt: bool,
}

pub fn input_lost_at_prompt(watch: &InputWatch) -> bool {
    watch.at_prompt && (watch.hangup_signal || (watch.started_on_terminal && watch.stdin_hung_up))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watch() -> InputWatch {
        InputWatch {
            hangup_signal: false,
            started_on_terminal: true,
            stdin_hung_up: false,
            at_prompt: true,
        }
    }

    #[test]
    fn live_terminal_keeps_input() {
        assert!(!input_lost_at_prompt(&watch()));
    }

    #[test]
    fn hangup_signal_loses_input() {
        assert!(input_lost_at_prompt(&InputWatch {
            hangup_signal: true,
            ..watch()
        }));
        assert!(input_lost_at_prompt(&InputWatch {
            hangup_signal: true,
            started_on_terminal: false,
            ..watch()
        }));
    }

    #[test]
    fn hung_up_terminal_loses_input() {
        assert!(input_lost_at_prompt(&InputWatch {
            stdin_hung_up: true,
            ..watch()
        }));
    }

    #[test]
    fn closed_non_terminal_stdin_keeps_input() {
        assert!(!input_lost_at_prompt(&InputWatch {
            started_on_terminal: false,
            stdin_hung_up: true,
            ..watch()
        }));
    }

    #[test]
    fn running_turn_keeps_input() {
        assert!(!input_lost_at_prompt(&InputWatch {
            hangup_signal: true,
            stdin_hung_up: true,
            at_prompt: false,
            ..watch()
        }));
    }
}
