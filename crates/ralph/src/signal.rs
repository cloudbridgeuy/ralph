use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn init() -> Result<(), ctrlc::Error> {
    ctrlc::set_handler(move || {
        INTERRUPTED.store(true, Ordering::SeqCst);
        eprintln!("\nInterrupted. Cleaning up...");
    })
}

#[inline]
pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}
