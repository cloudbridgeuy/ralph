use super::{
    CODEX_ANSWER_DIR, CODEX_CLOSE_DIR, CODEX_ERROR_DIR, CODEX_INTERRUPT_DIR,
    CODEX_INTERRUPT_RESULT_DIR, CODEX_OWNER_DIR, CODEX_PROMPT_DIR, CODEX_SESSION_DIR,
    CODEX_SPAWN_DIR, CURSOR_ANSWER_DIR, CURSOR_CLOSE_DIR, CURSOR_ERROR_DIR, CURSOR_INTERRUPT_DIR,
    CURSOR_INTERRUPT_RESULT_DIR, CURSOR_OWNER_DIR, CURSOR_PROMPT_DIR, CURSOR_SESSION_DIR,
    CURSOR_SPAWN_DIR, OPENCODE_ANSWER_DIR, OPENCODE_CLOSE_DIR, OPENCODE_ERROR_DIR,
    OPENCODE_HEALTHY_DIR, OPENCODE_INTERRUPT_DIR, OPENCODE_INTERRUPT_RESULT_DIR,
    OPENCODE_OWNER_DIR, OPENCODE_PREFLIGHT_DIR, OPENCODE_PROMPT_DIR, OPENCODE_SESSION_DIR,
    OPENCODE_SPAWN_DIR,
};
use std::path::{Path, PathBuf};

const CURSOR_DIRS: [&str; 9] = [
    CURSOR_SPAWN_DIR,
    CURSOR_CLOSE_DIR,
    CURSOR_ERROR_DIR,
    CURSOR_OWNER_DIR,
    CURSOR_ANSWER_DIR,
    CURSOR_PROMPT_DIR,
    CURSOR_INTERRUPT_DIR,
    CURSOR_INTERRUPT_RESULT_DIR,
    CURSOR_SESSION_DIR,
];

const CODEX_DIRS: [&str; 9] = [
    CODEX_SPAWN_DIR,
    CODEX_CLOSE_DIR,
    CODEX_ERROR_DIR,
    CODEX_OWNER_DIR,
    CODEX_ANSWER_DIR,
    CODEX_PROMPT_DIR,
    CODEX_INTERRUPT_DIR,
    CODEX_INTERRUPT_RESULT_DIR,
    CODEX_SESSION_DIR,
];

const OPENCODE_DIRS: [&str; 9] = [
    OPENCODE_SPAWN_DIR,
    OPENCODE_CLOSE_DIR,
    OPENCODE_ERROR_DIR,
    OPENCODE_OWNER_DIR,
    OPENCODE_ANSWER_DIR,
    OPENCODE_PROMPT_DIR,
    OPENCODE_INTERRUPT_DIR,
    OPENCODE_INTERRUPT_RESULT_DIR,
    OPENCODE_SESSION_DIR,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffFiles {
    pub spawn: PathBuf,
    pub close: PathBuf,
    pub error: PathBuf,
    pub owner: PathBuf,
    pub answer: PathBuf,
    pub prompt: PathBuf,
    pub interrupt: PathBuf,
    pub interrupt_result: PathBuf,
    pub session: PathBuf,
}

impl HandoffFiles {
    pub fn cursor(store: &Path, id: &str) -> Self {
        Self::in_dirs(store, id, CURSOR_DIRS)
    }

    pub fn codex(store: &Path, id: &str) -> Self {
        Self::in_dirs(store, id, CODEX_DIRS)
    }

    pub fn opencode(store: &Path, id: &str) -> Self {
        Self::in_dirs(store, id, OPENCODE_DIRS)
    }

    pub fn opencode_preflight(store: &Path, id: &str) -> PathBuf {
        store.join(OPENCODE_PREFLIGHT_DIR).join(id)
    }

    pub fn opencode_healthy(store: &Path, id: &str) -> PathBuf {
        store.join(OPENCODE_HEALTHY_DIR).join(id)
    }

    fn in_dirs(store: &Path, id: &str, dirs: [&str; 9]) -> Self {
        let [spawn, close, error, owner, answer, prompt, interrupt, interrupt_result, session] =
            dirs.map(|dir| store.join(dir).join(id));
        Self {
            spawn,
            close,
            error,
            owner,
            answer,
            prompt,
            interrupt,
            interrupt_result,
            session,
        }
    }

    pub fn all(&self) -> [&Path; 8] {
        [
            &self.spawn,
            &self.close,
            &self.error,
            &self.owner,
            &self.answer,
            &self.prompt,
            &self.interrupt,
            &self.interrupt_result,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_keep_their_on_disk_directories() {
        let cursor = HandoffFiles::cursor(Path::new("/s"), "A");
        assert_eq!(cursor.owner, PathBuf::from("/s/cursor/A"));
        assert_eq!(cursor.session, PathBuf::from("/s/cursor-session/A"));
        let codex = HandoffFiles::codex(Path::new("/s"), "A");
        assert_eq!(codex.owner, PathBuf::from("/s/codex/A"));
        assert_eq!(codex.session, PathBuf::from("/s/codex-session/A"));
        let opencode = HandoffFiles::opencode(Path::new("/s"), "A");
        assert_eq!(opencode.owner, PathBuf::from("/s/opencode-owner/A"));
        assert_eq!(opencode.session, PathBuf::from("/s/opencode-session/A"));
        assert_eq!(
            HandoffFiles::opencode_preflight(Path::new("/s"), "A"),
            PathBuf::from("/s/opencode-preflight/A")
        );
        assert!(!opencode.all().contains(&opencode.session.as_path()));
    }
}
