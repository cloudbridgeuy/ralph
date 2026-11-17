#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleSignal {
    CtrlC,
    CtrlD,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleStep {
    Closed,
    ClearDraft,
    Quit,
}

pub fn idle_step(signal: IdleSignal, draft_empty: bool, terminated: bool) -> IdleStep {
    match (terminated, signal, draft_empty) {
        (true, _, _) => IdleStep::Quit,
        (false, IdleSignal::CtrlC, false) => IdleStep::ClearDraft,
        (false, _, _) => IdleStep::Closed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopExit {
    IdleClose,
    ShellCell,
    HeldQuit,
    SwitchRefused,
    SwitchSaveFailed,
    ThreadOpRefused,
    TurnExit,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    Quit,
    Closed,
}

pub fn exit_kind(exit: LoopExit) -> ExitKind {
    match exit {
        LoopExit::IdleClose => ExitKind::Closed,
        LoopExit::ShellCell
        | LoopExit::HeldQuit
        | LoopExit::SwitchRefused
        | LoopExit::SwitchSaveFailed
        | LoopExit::ThreadOpRefused
        | LoopExit::TurnExit
        | LoopExit::History => ExitKind::Quit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_with_text_clears_the_draft() {
        assert_eq!(
            idle_step(IdleSignal::CtrlC, false, false),
            IdleStep::ClearDraft
        );
    }

    #[test]
    fn ctrl_c_on_an_empty_prompt_closes() {
        assert_eq!(idle_step(IdleSignal::CtrlC, true, false), IdleStep::Closed);
    }

    #[test]
    fn ctrl_d_closes() {
        assert_eq!(idle_step(IdleSignal::CtrlD, true, false), IdleStep::Closed);
    }

    #[test]
    fn a_hangup_seen_as_ctrl_d_quits() {
        assert_eq!(idle_step(IdleSignal::CtrlD, true, true), IdleStep::Quit);
    }

    #[test]
    fn a_hangup_seen_as_ctrl_c_quits() {
        assert_eq!(idle_step(IdleSignal::CtrlC, true, true), IdleStep::Quit);
        assert_eq!(idle_step(IdleSignal::CtrlC, false, true), IdleStep::Quit);
    }

    #[test]
    fn only_an_idle_close_is_closed() {
        assert_eq!(exit_kind(LoopExit::IdleClose), ExitKind::Closed);
        for exit in [
            LoopExit::ShellCell,
            LoopExit::HeldQuit,
            LoopExit::SwitchRefused,
            LoopExit::SwitchSaveFailed,
            LoopExit::ThreadOpRefused,
            LoopExit::TurnExit,
            LoopExit::History,
        ] {
            assert_eq!(exit_kind(exit), ExitKind::Quit, "{exit:?}");
        }
    }
}
