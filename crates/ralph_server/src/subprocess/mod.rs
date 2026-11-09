use std::process::Child;

mod groups;
mod timed;

pub(crate) use groups::Registered;
pub use groups::{kill_registered_groups, register_group, unregister_group};
pub use timed::{timed_output, TimedOutput};

pub fn kill_process_group(child: &mut Child) {
    signal_group(child);
    let _ = child.wait();
}

pub fn signal_group(child: &Child) {
    let pid = child.id() as libc::pid_t;
    unsafe { libc::killpg(pid, libc::SIGKILL) };
}
