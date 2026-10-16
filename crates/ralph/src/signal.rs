use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::flag::register;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

static INTERRUPTED: LazyLock<Arc<AtomicBool>> = LazyLock::new(|| Arc::new(AtomicBool::new(false)));
static TERMINATED: LazyLock<Arc<AtomicBool>> = LazyLock::new(|| Arc::new(AtomicBool::new(false)));

pub fn init() -> io::Result<()> {
    register(SIGINT, Arc::clone(&INTERRUPTED))?;
    register(SIGTERM, Arc::clone(&TERMINATED))?;
    register(SIGHUP, Arc::clone(&TERMINATED))?;
    Ok(())
}

pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst) || is_terminated()
}

pub fn is_terminated() -> bool {
    TERMINATED.load(Ordering::SeqCst)
}

pub fn interrupt_flag() -> Arc<AtomicBool> {
    Arc::clone(&INTERRUPTED)
}

/// Backstop for `ralph chat`: if ralph ends up in a background process group,
/// the raw-mode switch would raise SIGTTOU and suspend the whole chat. Ignoring
/// it makes the terminal write fail or succeed instead of stopping the process.
pub fn ignore_sigttou() {
    // SAFETY: setting a signal disposition to SIG_IGN is async-signal-safe and
    // installs no handler code.
    unsafe {
        libc::signal(libc::SIGTTOU, libc::SIG_IGN);
    }
}

/// An ignored SIGTTOU survives exec, so a child of `ralph chat` (claude -p, a
/// cell) would not stop on background terminal writes the way it should. This
/// puts the default disposition back in the child before exec.
pub fn default_sigttou_in(command: &mut Command) {
    // SAFETY: signal(2) is async-signal-safe, so it may run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::signal(libc::SIGTTOU, libc::SIG_DFL) == libc::SIG_ERR {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}
