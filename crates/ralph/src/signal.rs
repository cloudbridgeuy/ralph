use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::flag::register;
use std::io;
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
