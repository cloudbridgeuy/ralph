pub mod serve;
pub mod sweep;

use ralph_core::chat_socket::{chat_files_dir, sidecar_name, socket_file_name, StoreKey};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SocketFiles {
    pub files_dir: PathBuf,
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
        Self {
            files_dir: chat_files_dir(place.store, pid),
            socket: place.root.join(socket_file_name(&place.key, pid)),
            sidecar: place.store.join(sidecar_name(pid)),
        }
    }

    pub fn remove(&self) {
        fs::remove_file(&self.socket).ok();
        fs::remove_dir_all(&self.files_dir).ok();
        fs::remove_file(&self.sidecar).ok();
    }
}
