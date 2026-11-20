use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use ralph_core::chat_socket::check_socket_path;

use crate::wire::store::{current_uid, ensure_store_root};

pub struct ServeSocket {
    listener: UnixListener,
    dir: PathBuf,
    uid: u32,
}

impl ServeSocket {
    pub fn bind(root: &Path, socket: &Path) -> io::Result<Self> {
        ensure_store_root(root)?;
        check_socket_path(root, socket).map_err(io::Error::other)?;
        let dir = socket
            .parent()
            .ok_or_else(|| io::Error::other("serve socket path has no parent directory"))?
            .to_path_buf();
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
            }
            Err(e) => return Err(e),
        }
        match fs::remove_file(socket) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = match open_listener(socket) {
            Ok(listener) => listener,
            Err(e) => {
                let _ = fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        Ok(Self {
            listener,
            dir,
            uid: current_uid(),
        })
    }

    pub fn accept(&self) -> io::Result<Option<UnixStream>> {
        let stream = match self.listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(e) => return Err(e),
        };
        stream.set_nonblocking(false)?;
        if peer_uid(&stream)? != self.uid {
            return Ok(None);
        }
        Ok(Some(stream))
    }
}

impl Drop for ServeSocket {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn open_listener(socket: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    use std::os::fd::AsRawFd;
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(cred.uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::time::Duration;

    fn store_root(tmp: &tempfile::TempDir) -> PathBuf {
        tmp.path().join("r")
    }

    fn paths(root: &Path) -> PathBuf {
        root.join("serve-abcdef012345").join("serve.sock")
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & 0o777
    }

    #[test]
    fn bind_sets_private_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = store_root(&tmp);
        let socket = paths(&root);
        let _bound = ServeSocket::bind(&root, &socket).unwrap();
        assert_eq!(mode(&socket), 0o600);
        assert_eq!(mode(socket.parent().unwrap()), 0o700);
    }

    #[test]
    fn bind_replaces_stale_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = store_root(&tmp);
        let socket = paths(&root);
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(socket.parent().unwrap())
            .unwrap();
        fs::write(&socket, b"stale").unwrap();
        let bound = ServeSocket::bind(&root, &socket);
        assert!(bound.is_ok());
        assert_eq!(mode(&socket), 0o600);
    }

    #[test]
    fn bind_tightens_existing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = store_root(&tmp);
        let socket = paths(&root);
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(socket.parent().unwrap())
            .unwrap();
        fs::set_permissions(socket.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();
        let _bound = ServeSocket::bind(&root, &socket).unwrap();
        assert_eq!(mode(socket.parent().unwrap()), 0o700);
    }

    #[test]
    fn drop_removes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = store_root(&tmp);
        let socket = paths(&root);
        let bound = ServeSocket::bind(&root, &socket).unwrap();
        drop(bound);
        assert!(!socket.parent().unwrap().exists());
    }

    #[test]
    fn accept_returns_pending_client() {
        let tmp = tempfile::tempdir().unwrap();
        let root = store_root(&tmp);
        let socket = paths(&root);
        let bound = ServeSocket::bind(&root, &socket).unwrap();
        let _client = UnixStream::connect(&socket).unwrap();
        let mut accepted = None;
        for _ in 0..50 {
            accepted = bound.accept().unwrap();
            if accepted.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(accepted.is_some());
    }

    #[test]
    fn accept_without_client_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let root = store_root(&tmp);
        let bound = ServeSocket::bind(&root, &paths(&root)).unwrap();
        assert!(bound.accept().unwrap().is_none());
    }

    #[test]
    fn peer_uid_matches_effective_uid() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&a).unwrap(), current_uid());
    }
}
