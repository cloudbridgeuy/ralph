use ralph_core::chat_socket::{runtime_dir_name, sidecar_name, socket_file_name, StoreKey};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SocketFiles {
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub sidecar: PathBuf,
}

pub struct SocketPlace<'a> {
    pub store: &'a Path,
    pub root: &'a Path,
    pub key: StoreKey,
}

impl SocketFiles {
    pub fn new(place: &SocketPlace, pid: u32) -> Self {
        let private = place.root.join(runtime_dir_name(&place.key, pid));
        Self {
            socket: private.join(socket_file_name()),
            dir: private,
            sidecar: place.store.join(sidecar_name(pid)),
        }
    }

    pub fn remove(&self) {
        fs::remove_dir_all(&self.dir).ok();
        fs::remove_file(&self.sidecar).ok();
    }
}
