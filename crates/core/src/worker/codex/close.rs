use std::time::Duration;

pub const CODEX_CLOSE_TURN_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexCloseStep {
    WindDown,
    AwaitTurn,
    SnapshotTree,
    DropStdin,
    StopGroup,
    SweepTree,
    RemoveOwner,
}

pub fn codex_close_plan(turn_active: bool) -> Vec<CodexCloseStep> {
    let mut steps = vec![CodexCloseStep::WindDown];
    if turn_active {
        steps.push(CodexCloseStep::AwaitTurn);
    }
    steps.extend([
        CodexCloseStep::SnapshotTree,
        CodexCloseStep::DropStdin,
        CodexCloseStep::StopGroup,
        CodexCloseStep::SweepTree,
        CodexCloseStep::RemoveOwner,
    ]);
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_is_snapshotted_before_stdin_drops_and_before_any_signal() {
        for active in [true, false] {
            let plan = codex_close_plan(active);
            let at = |step| plan.iter().position(|s| *s == step);
            assert!(at(CodexCloseStep::SnapshotTree) < at(CodexCloseStep::DropStdin));
            assert!(at(CodexCloseStep::DropStdin) < at(CodexCloseStep::StopGroup));
            assert!(at(CodexCloseStep::StopGroup) < at(CodexCloseStep::SweepTree));
            assert_eq!(at(CodexCloseStep::WindDown), Some(0));
            assert_eq!(at(CodexCloseStep::AwaitTurn).is_some(), active);
        }
    }
}
