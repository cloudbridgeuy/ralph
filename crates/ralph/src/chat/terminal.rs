use std::mem::MaybeUninit;

const STDIN: libc::c_int = libc::STDIN_FILENO;

pub(super) fn partial_mode(mut mode: libc::termios) -> libc::termios {
    mode.c_lflag &= !(libc::ICANON | libc::ECHO);
    mode.c_iflag &= !libc::IXON;
    mode.c_cc[libc::VMIN] = 1;
    mode.c_cc[libc::VTIME] = 0;
    mode
}

pub(super) struct PartialMode {
    original: libc::termios,
}

impl PartialMode {
    pub(super) fn enter() -> Option<Self> {
        let mut slot = MaybeUninit::<libc::termios>::uninit();
        let original = unsafe {
            (libc::tcgetattr(STDIN, slot.as_mut_ptr()) == 0).then(|| slot.assume_init())?
        };
        let partial = partial_mode(original);
        (unsafe { libc::tcsetattr(STDIN, libc::TCSANOW, &partial) } == 0)
            .then_some(Self { original })
    }
}

impl Drop for PartialMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcflush(STDIN, libc::TCIFLUSH);
            libc::tcsetattr(STDIN, libc::TCSANOW, &self.original);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cooked() -> libc::termios {
        let mut mode: libc::termios = unsafe { std::mem::zeroed() };
        mode.c_lflag = libc::ICANON | libc::ECHO | libc::ISIG | libc::IEXTEN;
        mode.c_iflag = libc::IXON | libc::ICRNL;
        mode.c_oflag = libc::OPOST;
        mode.c_cc[libc::VMIN] = 4;
        mode.c_cc[libc::VTIME] = 9;
        mode
    }

    #[test]
    fn partial_mode_clears_canonical_echo_and_flow_control() {
        let mode = partial_mode(cooked());
        assert_eq!(mode.c_lflag & libc::ICANON, 0);
        assert_eq!(mode.c_lflag & libc::ECHO, 0);
        assert_eq!(mode.c_iflag & libc::IXON, 0);
    }

    #[test]
    fn partial_mode_keeps_signals_and_output_processing() {
        let mode = partial_mode(cooked());
        assert_ne!(mode.c_lflag & libc::ISIG, 0);
        assert_ne!(mode.c_oflag & libc::OPOST, 0);
        assert_ne!(mode.c_iflag & libc::ICRNL, 0);
    }

    #[test]
    fn partial_mode_reads_one_byte_without_timeout() {
        let mode = partial_mode(cooked());
        assert_eq!(mode.c_cc[libc::VMIN], 1);
        assert_eq!(mode.c_cc[libc::VTIME], 0);
    }
}
