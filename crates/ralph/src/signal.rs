use ralph_core::cancel::CancelToken;
use ralph_server::warn::warn;
use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::flag::register;
use signal_hook::low_level::unregister;
use signal_hook::SigId;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Once};

static TERMINATED: LazyLock<Arc<AtomicBool>> = LazyLock::new(|| Arc::new(AtomicBool::new(false)));
static SIGINT_SINK: Once = Once::new();

pub struct Armed(SigId);

impl Drop for Armed {
    fn drop(&mut self) {
        unregister(self.0);
    }
}

pub fn arm(token: &CancelToken) -> Option<Armed> {
    register(SIGINT, token.flag())
        .map(Armed)
        .inspect_err(|e| warn(format!("Failed to arm the Ctrl-C handler: {e}")))
        .ok()
}

pub fn init() -> io::Result<()> {
    install_sigint_sink()?;
    register(SIGTERM, Arc::clone(&TERMINATED))?;
    register(SIGHUP, Arc::clone(&TERMINATED))?;
    Ok(())
}

pub fn init_serve() -> io::Result<()> {
    for signal in [SIGINT, SIGTERM, SIGHUP] {
        register(signal, Arc::clone(&TERMINATED))?;
    }
    Ok(())
}

// The sink's SigId is dropped on purpose and the Once never resets: with no
// SIGINT action left installed, a Ctrl-C between turns would kill ralph.
fn install_sigint_sink() -> io::Result<()> {
    let mut installed = Ok(());
    SIGINT_SINK.call_once(|| {
        installed = register(SIGINT, Arc::new(AtomicBool::new(false))).map(drop);
    });
    installed
}

pub fn is_terminated() -> bool {
    TERMINATED.load(Ordering::SeqCst)
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
