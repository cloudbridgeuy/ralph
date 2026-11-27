use std::io::Result as IoResult;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr};

use crate::connection::Connection;
#[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
use crate::ssl::SslStream;

pub(crate) enum Stream {
    Http(Connection),
    #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
    Https(SslStream),
}

impl Stream {
    fn try_clone(&self) -> IoResult<Stream> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.try_clone().map(Stream::Http),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => Ok(Stream::Https(ssl_stream.clone())),
        }
    }
}

impl From<Connection> for Stream {
    fn from(tcp_stream: Connection) -> Self {
        Stream::Http(tcp_stream)
    }
}

impl Stream {
    fn secure(&self) -> bool {
        match self {
            Stream::Http(_) => false,
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(_) => true,
        }
    }

    fn peer_addr(&mut self) -> IoResult<Option<SocketAddr>> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.peer_addr(),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.peer_addr(),
        }
    }

    fn shutdown(&mut self, how: Shutdown) -> IoResult<()> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.shutdown(how),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.shutdown(how),
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.read(buf),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.write(buf),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.write(buf),
        }
    }

    fn flush(&mut self) -> IoResult<()> {
        match self {
            Stream::Http(tcp_stream) => tcp_stream.flush(),
            #[cfg(any(feature = "ssl-openssl", feature = "ssl-rustls"))]
            Stream::Https(ssl_stream) => ssl_stream.flush(),
        }
    }
}

pub struct RefinedTcpStream {
    stream: Stream,
    close_read: bool,
    close_write: bool,
}

impl RefinedTcpStream {
    pub(crate) fn new<S>(stream: S) -> IoResult<(RefinedTcpStream, RefinedTcpStream)>
    where
        S: Into<Stream>,
    {
        Self::split_with(stream.into(), Stream::try_clone)
    }

    fn split_with(
        stream: Stream,
        try_clone: impl FnOnce(&Stream) -> IoResult<Stream>,
    ) -> IoResult<(RefinedTcpStream, RefinedTcpStream)> {
        let (read, write) = (try_clone(&stream)?, stream);

        let read = RefinedTcpStream {
            stream: read,
            close_read: true,
            close_write: false,
        };

        let write = RefinedTcpStream {
            stream: write,
            close_read: false,
            close_write: true,
        };

        Ok((read, write))
    }

    pub(crate) fn handle(&self) -> IoResult<RefinedTcpStream> {
        self.handle_with(Stream::try_clone)
    }

    fn handle_with(
        &self,
        try_clone: impl FnOnce(&Stream) -> IoResult<Stream>,
    ) -> IoResult<RefinedTcpStream> {
        Ok(RefinedTcpStream {
            stream: try_clone(&self.stream)?,
            close_read: false,
            close_write: false,
        })
    }

    pub(crate) fn shutdown_both(&mut self) {
        self.stream.shutdown(Shutdown::Both).ok();
    }

    /// Returns true if this struct wraps around a secure connection.
    #[inline]
    pub(crate) fn secure(&self) -> bool {
        self.stream.secure()
    }

    pub(crate) fn peer_addr(&mut self) -> IoResult<Option<SocketAddr>> {
        self.stream.peer_addr()
    }
}

impl Drop for RefinedTcpStream {
    fn drop(&mut self) {
        if self.close_read {
            self.stream.shutdown(Shutdown::Read).ok();
        }

        if self.close_write {
            self.stream.shutdown(Shutdown::Write).ok();
        }
    }
}

impl Read for RefinedTcpStream {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        self.stream.read(buf)
    }
}

impl Write for RefinedTcpStream {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod test {
    use super::{RefinedTcpStream, Stream};
    use crate::connection::Connection;
    use std::io::{Error, ErrorKind, Result as IoResult};
    use std::net::{TcpListener, TcpStream};

    fn server_stream() -> Stream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        Stream::from(Connection::from(server))
    }

    fn failing_clone(_: &Stream) -> IoResult<Stream> {
        Err(Error::from(ErrorKind::OutOfMemory))
    }

    #[test]
    fn split_returns_the_clone_error() {
        let err = RefinedTcpStream::split_with(server_stream(), failing_clone)
            .err()
            .unwrap();
        assert_eq!(err.kind(), ErrorKind::OutOfMemory);
    }

    #[test]
    fn split_returns_both_halves_when_the_clone_works() {
        let (read, write) = RefinedTcpStream::split_with(server_stream(), Stream::try_clone)
            .ok()
            .unwrap();
        assert!(read.close_read && !read.close_write);
        assert!(write.close_write && !write.close_read);
    }

    #[test]
    fn handle_returns_the_clone_error() {
        let (read, _write) = RefinedTcpStream::new(server_stream()).ok().unwrap();
        let err = read.handle_with(failing_clone).err().unwrap();
        assert_eq!(err.kind(), ErrorKind::OutOfMemory);
    }
}
