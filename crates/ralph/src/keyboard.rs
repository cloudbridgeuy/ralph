//! Keyboard action types for subprocess execution.
//!
//! This module carries the [`RunKeyAction`] enum, which represents actions that
//! can be signalled to the subprocess loop. Keyboard polling infrastructure
//! (`RawModeGuard`, `check_for_run_action`, `classify_run_key`) was removed in
//! S3 of the dismantle-keybindings plan. The enum itself will be removed in S4b
//! once all callers are updated.

/// Keyboard actions available during the main run loop.
///
/// These variants represent user-initiated control signals. After S3 the
/// subprocess loop no longer polls for these keys — `key_action` in
/// [`crate::subprocess::SpinnerSubprocessOutcome`] is always `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKeyAction {
    /// No key was pressed.
    None,
    /// Soft stop: finish current iteration then exit (s key).
    SoftStop,
    /// Hard stop: immediately halt and save paused state (S key).
    HardStop,
    /// Toggle pause/resume (p key).
    Pause,
    /// Interrupt: immediately halt (Ctrl+C).
    Interrupt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_key_action_enum_variants() {
        assert_ne!(RunKeyAction::None, RunKeyAction::SoftStop);
        assert_ne!(RunKeyAction::SoftStop, RunKeyAction::HardStop);
        assert_ne!(RunKeyAction::HardStop, RunKeyAction::Pause);
    }
}
