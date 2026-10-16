use crate::config::ChangesSection;
use crate::signal;
use crate::warn::warn;
use ralph_core::diff_command::diff_command;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

/// Run `git diff` through the configured pager and wait.
///
/// Chat reaches this only after `Reedline::read_line` returns. That function
/// enables raw mode and disables it before returning
/// (`vendor/reedline/src/engine.rs`), so the pager already has a normal
/// terminal. The next `read_line` redraws the prompt.
pub(super) fn run(changes: &ChangesSection, extra: &[String]) {
    let argv = diff_command(changes.pager(), &changes.diff_args, extra);
    let Some((program, args)) = argv.split_first() else {
        return;
    };
    // Ctrl-C during the pager sets the chat interrupt flag. Clear it so the
    // next turn is not treated as cancelled. exec resets the child's handler.
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    let run = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    if let Err(error) = run {
        warn(format!("failed to run git diff: {error}"));
    }
}
