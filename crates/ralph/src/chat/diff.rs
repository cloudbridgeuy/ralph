use crate::chat::changes::resolve_diff;
use crate::config::ChangesSection;
use crate::signal;
use crate::warn::warn;
use ralph_core::changes::NO_CHANGES;
use ralph_core::diff_command::{diff_command, diff_quiet_command, range_is_empty, DiffTarget};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;

pub(super) struct DiffContext<'a> {
    pub changes: &'a ChangesSection,
    pub store: &'a Path,
}

/// Run `git diff` through the configured pager and wait.
///
/// Chat reaches this only after `Reedline::read_line` returns. That function
/// enables raw mode and disables it before returning
/// (`vendor/reedline/src/engine.rs`), so the pager already has a normal
/// terminal. The next `read_line` redraws the prompt.
pub(super) fn run(ctx: &DiffContext, target: DiffTarget) {
    match target {
        DiffTarget::Now(extra) => spawn(ctx.changes, &[], &extra, |_| {}),
        DiffTarget::Range { scope, extra } => {
            let resolved = std::env::current_dir()
                .map_err(|e| format!("Could not read the working directory: {e}"))
                .and_then(|cwd| resolve_diff(ctx.store, &cwd, scope));
            match resolved {
                Ok((snapshotter, range, note)) => {
                    if let Some(note) = note {
                        println!("{note}");
                    }
                    let revs: Vec<String> = std::iter::once(range.from).chain(range.to).collect();
                    if probe_is_empty(ctx.changes, &revs, &extra, |command| {
                        snapshotter.side_env(command)
                    }) {
                        println!("{NO_CHANGES}");
                        return;
                    }
                    spawn(ctx.changes, &revs, &extra, |command| {
                        snapshotter.side_env(command)
                    });
                }
                Err(message) => println!("{message}"),
            }
        }
    }
}

fn probe_is_empty(
    changes: &ChangesSection,
    revs: &[String],
    extra: &[String],
    configure: impl FnOnce(&mut Command),
) -> bool {
    let argv = diff_quiet_command(revs, &changes.diff_args, extra);
    let Some((program, args)) = argv.split_first() else {
        return false;
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    configure(&mut command);
    range_is_empty(command.status().ok().and_then(|status| status.code()))
}

fn spawn(
    changes: &ChangesSection,
    revs: &[String],
    extra: &[String],
    configure: impl FnOnce(&mut Command),
) {
    let argv = diff_command(changes.pager(), revs, &changes.diff_args, extra);
    let Some((program, args)) = argv.split_first() else {
        return;
    };
    // Ctrl-C during the pager sets the chat interrupt flag. Clear it so the
    // next turn is not treated as cancelled. exec resets the child's handler.
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    configure(&mut command);
    let run = command.status();
    signal::interrupt_flag().store(false, Ordering::SeqCst);
    if let Err(error) = run {
        warn(format!("failed to run git diff: {error}"));
    }
}
