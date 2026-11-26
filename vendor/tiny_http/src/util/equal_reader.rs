use std::io::Read;
use std::io::Result as IoResult;
use std::sync::mpsc::channel;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// A `Reader` that reads exactly the number of bytes from a sub-reader.
///
/// If the limit is reached, it returns EOF. If the limit is not reached
/// when the destructor is called, the remaining bytes will be read and
/// thrown away.
pub struct EqualReader<R>
where
    R: Read,
{
    reader: R,
    size: usize,
    last_read_signal: Sender<IoResult<()>>,
}

impl<R> EqualReader<R>
where
    R: Read,
{
    pub fn new(reader: R, size: usize) -> (EqualReader<R>, Receiver<IoResult<()>>) {
        let (tx, rx) = channel();

        let r = EqualReader {
            reader,
            size,
            last_read_signal: tx,
        };

        (r, rx)
    }
}

impl<R> Read for EqualReader<R>
where
    R: Read,
{
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        if self.size == 0 {
            return Ok(0);
        }

        let buf = if buf.len() < self.size {
            buf
        } else {
            &mut buf[..self.size]
        };

        match self.reader.read(buf) {
            Ok(len) => {
                self.size = self.size.saturating_sub(len);
                Ok(len)
            }
            err @ Err(_) => err,
        }
    }
}

const DRAIN_BUDGET: usize = 64 * 1024;
const DRAIN_DEADLINE: Duration = Duration::from_secs(2);

impl<R> EqualReader<R>
where
    R: Read,
{
    fn drain(&mut self, budget: usize, deadline: Duration) {
        let started = Instant::now();
        let mut remaining_to_read = self.size.min(budget);

        let mut buf = [0u8; 8192];

        while remaining_to_read > 0 && started.elapsed() < deadline {
            let chunk = remaining_to_read.min(buf.len());

            match self.reader.read(&mut buf[..chunk]) {
                Err(e) => {
                    self.last_read_signal.send(Err(e)).ok();
                    break;
                }
                Ok(0) => {
                    self.last_read_signal.send(Ok(())).ok();
                    break;
                }
                Ok(other) => {
                    remaining_to_read = remaining_to_read.saturating_sub(other);
                }
            }
        }
    }
}

impl<R> Drop for EqualReader<R>
where
    R: Read,
{
    fn drop(&mut self) {
        self.drain(DRAIN_BUDGET, DRAIN_DEADLINE);
    }
}

#[cfg(test)]
mod tests {
    use super::EqualReader;
    use std::io::Read;

    #[test]
    fn test_limit() {
        use std::io::Cursor;

        let mut org_reader = Cursor::new("hello world".to_string().into_bytes());

        {
            let (mut equal_reader, _) = EqualReader::new(org_reader.by_ref(), 5);

            let mut string = String::new();
            equal_reader.read_to_string(&mut string).unwrap();
            assert_eq!(string, "hello");
        }

        let mut string = String::new();
        org_reader.read_to_string(&mut string).unwrap();
        assert_eq!(string, " world");
    }

    #[test]
    fn test_not_enough() {
        use std::io::Cursor;

        let mut org_reader = Cursor::new("hello world".to_string().into_bytes());

        {
            let (mut equal_reader, _) = EqualReader::new(org_reader.by_ref(), 5);

            let mut vec = [0];
            equal_reader.read_exact(&mut vec).unwrap();
            assert_eq!(vec[0], b'h');
        }

        let mut string = String::new();
        org_reader.read_to_string(&mut string).unwrap();
        assert_eq!(string, " world");
    }

    struct RecordingReader {
        data: std::io::Cursor<Vec<u8>>,
        largest_request: usize,
    }

    impl Read for RecordingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.largest_request = self.largest_request.max(buf.len());
            self.data.read(buf)
        }
    }

    #[test]
    fn drop_drain_requests_bounded_buffers() {
        let mut inner = RecordingReader {
            data: std::io::Cursor::new(vec![7u8; 4096]),
            largest_request: 0,
        };

        {
            let (_equal_reader, _) = EqualReader::new(&mut inner, 4usize << 30);
        }

        assert!(inner.largest_request > 0);
        assert!(inner.largest_request <= 8192);
    }

    #[test]
    fn drop_drain_consumes_remaining_body_and_signals_eof_once() {
        let mut inner = RecordingReader {
            data: std::io::Cursor::new(vec![1u8; 20000]),
            largest_request: 0,
        };

        let rx = {
            let (_equal_reader, rx) = EqualReader::new(&mut inner, 20000);
            rx
        };

        assert_eq!(inner.data.position(), 20000);
        assert!(rx.try_recv().is_err());
    }

    struct EndlessReader {
        total: usize,
    }

    impl Read for EndlessReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            buf.fill(9);
            self.total += buf.len();
            Ok(buf.len())
        }
    }

    struct SlowReader {
        total: usize,
    }

    impl Read for SlowReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(std::time::Duration::from_millis(20));
            buf[0] = 1;
            self.total += 1;
            Ok(1)
        }
    }

    #[test]
    fn drop_drain_stops_after_the_byte_budget() {
        let mut inner = EndlessReader { total: 0 };

        {
            let (_equal_reader, _) = EqualReader::new(&mut inner, 1usize << 30);
        }

        assert!(inner.total > 0);
        assert!(inner.total <= 65536);
    }

    #[test]
    fn drain_stops_after_the_deadline() {
        let mut inner = SlowReader { total: 0 };
        let started = std::time::Instant::now();

        {
            let (equal_reader, _) = EqualReader::new(&mut inner, 200);
            let mut equal_reader = std::mem::ManuallyDrop::new(equal_reader);
            equal_reader.drain(65536, std::time::Duration::from_millis(50));
        }

        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(inner.total > 0);
        assert!(inner.total < 20);
    }

    struct LyingReader;

    impl Read for LyingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            Ok(buf.len() + 10)
        }
    }

    #[test]
    fn drop_drain_survives_a_reader_that_returns_more_than_asked() {
        let (equal_reader, _) = EqualReader::new(LyingReader, 100);
        drop(equal_reader);
    }

    #[test]
    fn read_survives_a_reader_that_returns_more_than_asked() {
        let (mut equal_reader, _) = EqualReader::new(LyingReader, 5);
        let mut buf = [0u8; 8];

        assert_eq!(equal_reader.read(&mut buf).unwrap(), 15);
        assert_eq!(equal_reader.read(&mut buf).unwrap(), 0);
    }
}
