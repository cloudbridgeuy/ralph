use super::*;

impl CodexHost {
    pub(super) fn deliver_queued(
        &self,
        id: &str,
        dir: fn(&HandoffFiles) -> &PathBuf,
        deliver: fn(&mut CodexWorker, String, &str),
    ) {
        let starting = locked(&self.starting).contains_key(id);
        if starting && !locked(&self.workers).contains_key(id) {
            return;
        }
        for entry in queued_entries(dir(&self.files(id))) {
            let name = entry
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let (Ok(text), Some(worker)) = (
                fs::read_to_string(&entry),
                locked(&self.workers).get_mut(id),
            ) {
                deliver(worker, text, &name);
            }
            fs::remove_file(&entry).ok();
        }
    }
}
