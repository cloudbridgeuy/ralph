use std::fs::File;

#[derive(Debug)]
pub struct ChatClaim(pub(super) File);

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
