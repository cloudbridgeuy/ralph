use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread;

mod groups;
mod timed;

pub(crate) use groups::Registered;
pub(crate) use groups::{kill_registered_groups, register_group, unregister_group};
pub(crate) use timed::{timed_output, TimedOutput};

pub fn kill_process_group(child: &mut Child) {
    signal_group(child);
    let _ = child.wait();
}

pub fn signal_group(child: &Child) {
    let pid = child.id() as libc::pid_t;
    unsafe { libc::killpg(pid, libc::SIGKILL) };
}

pub fn exited_unreaped(child: &Child) -> bool {
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let found = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id() as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    found != 0 || info.si_signo != 0
}

pub fn spawn_detached(command: &mut Command) -> io::Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    thread::spawn(move || child.wait().ok());
    Ok(())
}

pub fn detach_from_terminal(command: &mut Command) {
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}
