use std::mem::MaybeUninit;

const STDIN: libc::c_int = libc::STDIN_FILENO;
pub(super) const POLL_MS: libc::c_int = 50;
const READ_CHUNK: usize = 256;

pub(super) fn partial_mode(mut mode: libc::termios) -> libc::termios {
    mode.c_lflag &= !(libc::ICANON | libc::ECHO);
    mode.c_iflag &= !libc::IXON;
    mode.c_cc[libc::VMIN] = 1;
    mode.c_cc[libc::VTIME] = 0;
    mode
}

pub(crate) struct PartialMode {
    original: libc::termios,
}

impl PartialMode {
    pub(crate) fn enter() -> Option<Self> {
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
            libc::tcsetattr(STDIN, libc::TCSANOW, &self.original);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Input {
    Ready,
    Idle,
    Closed,
}

pub(super) fn classify_poll(result: libc::c_int, revents: libc::c_short) -> Input {
    if result <= 0 {
        Input::Idle
    } else if revents & libc::POLLIN != 0 {
        Input::Ready
    } else if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
        Input::Closed
    } else {
        Input::Idle
    }
}

pub(crate) fn poll_input(timeout_ms: libc::c_int) -> Input {
    let mut input = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe { libc::poll(&mut input, 1, timeout_ms) };
    classify_poll(result, input.revents)
}

pub(crate) fn read_input() -> Vec<u8> {
    let mut buffer = [0u8; READ_CHUNK];
    let count = unsafe { libc::read(libc::STDIN_FILENO, buffer.as_mut_ptr().cast(), buffer.len()) };
    usize::try_from(count)
        .map(|count| buffer[..count].to_vec())
        .unwrap_or_default()
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

    #[test]
    fn poll_with_data_is_ready_even_when_hung_up() {
        assert_eq!(classify_poll(1, libc::POLLIN), Input::Ready);
        assert_eq!(classify_poll(1, libc::POLLIN | libc::POLLHUP), Input::Ready);
    }

    #[test]
    fn poll_hangup_error_or_invalid_is_closed() {
        assert_eq!(classify_poll(1, libc::POLLHUP), Input::Closed);
        assert_eq!(classify_poll(1, libc::POLLERR), Input::Closed);
        assert_eq!(classify_poll(1, libc::POLLNVAL), Input::Closed);
    }

    #[test]
    fn poll_timeout_or_failure_is_idle() {
        assert_eq!(classify_poll(0, 0), Input::Idle);
        assert_eq!(classify_poll(-1, 0), Input::Idle);
    }
}
