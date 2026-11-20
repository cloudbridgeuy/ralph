use crate::cursor_owner::own_instance;
use crate::private_file::check_path;
use ralph_core::chat_memory::{holder_pid, next_lease};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug)]
pub struct ChatClaim(pub(super) File);

pub(super) fn claim_chat(dir: &Path) -> io::Result<Result<ChatClaim, Option<u32>>> {
    fs::create_dir_all(dir)?;
    let path = dir.join("CHAT");
    check_path(&path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    let locked = file.try_lock();
    if let Err(TryLockError::Error(e)) = locked {
        return Err(e);
    }
    let mut previous = String::new();
    file.read_to_string(&mut previous)?;
    if locked.is_err() {
        return Ok(Err(holder_pid(&previous)));
    }
    let pid = std::process::id();
    let holder = own_instance().map(str::to_string);
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    write!(file, "{}", next_lease(&previous, pid, holder).render())?;
    crate::history_log::clear_turn(dir);
    Ok(Ok(ChatClaim(file)))
}

impl Drop for ChatClaim {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod tests {
    use crate::chat::memory::Store;

    #[test]
    fn a_claim_releases_even_while_a_spawned_child_holds_its_fd() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let claim = store.claim_chat().unwrap().unwrap();
        let child_copy = claim.0.try_clone().unwrap();
        drop(claim);
        assert!(store.claim_chat().unwrap().is_ok());
        drop(child_copy);
    }
}
