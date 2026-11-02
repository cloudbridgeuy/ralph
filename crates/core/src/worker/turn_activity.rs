use super::{ActivityState, TurnEnd};

pub fn turn_end_activity(end: &TurnEnd) -> ActivityState {
    match end {
        TurnEnd::Failed { .. } => ActivityState::Failed,
        TurnEnd::Succeeded | TurnEnd::Interrupted => ActivityState::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_turn_end_is_failed_whether_or_not_it_is_transient() {
        for transient in [false, true] {
            let end = TurnEnd::Failed {
                message: "overloaded".into(),
                transient,
            };
            assert_eq!(turn_end_activity(&end), ActivityState::Failed);
        }
    }

    #[test]
    fn a_succeeded_turn_end_is_idle() {
        assert_eq!(turn_end_activity(&TurnEnd::Succeeded), ActivityState::Idle);
    }

    #[test]
    fn an_interrupted_turn_end_is_idle() {
        assert_eq!(
            turn_end_activity(&TurnEnd::Interrupted),
            ActivityState::Idle
        );
    }
}
