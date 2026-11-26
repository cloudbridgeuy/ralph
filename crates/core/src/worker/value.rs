use super::{capabilities, is_dashed_token, Feature, HarnessId, Support};
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorkerId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerIdError {
    Empty,
    NotDashed(String),
}

impl fmt::Display for WorkerIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Worker id is empty."),
            Self::NotDashed(text) => write!(
                f,
                "Worker id '{text}' may only use letters, digits and dashes."
            ),
        }
    }
}

impl std::error::Error for WorkerIdError {}

impl WorkerId {
    pub fn parse(text: &str) -> Result<Self, WorkerIdError> {
        if text.is_empty() {
            return Err(WorkerIdError::Empty);
        }
        if !is_dashed_token(text) {
            return Err(WorkerIdError::NotDashed(text.to_string()));
        }
        Ok(Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkerId {
    type Error = WorkerIdError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<WorkerId> for String {
    fn from(id: WorkerId) -> Self {
        id.0
    }
}

impl fmt::Display for WorkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Borrow<str> for WorkerId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef(PathBuf);

impl SessionRef {
    pub fn new(record: PathBuf) -> Self {
        Self(record)
    }

    pub fn record(&self) -> &Path {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerState {
    Running,
    Saved(SessionRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worker {
    id: WorkerId,
    harness: HarnessId,
    state: WorkerState,
}

impl Worker {
    pub fn running(id: WorkerId, harness: HarnessId) -> Self {
        Self {
            id,
            harness,
            state: WorkerState::Running,
        }
    }

    pub fn saved(id: WorkerId, harness: HarnessId, session: SessionRef) -> Result<Self, String> {
        if capabilities(harness).support(Feature::Resume) == Support::None {
            return Err(format!(
                "{} cannot resume a saved session.",
                harness.as_str()
            ));
        }
        Ok(Self {
            id,
            harness,
            state: WorkerState::Saved(session),
        })
    }

    pub fn id(&self) -> &WorkerId {
        &self.id
    }

    pub fn state(&self) -> &WorkerState {
        &self.state
    }

    pub fn resume_limit(&self) -> Option<&'static str> {
        match (
            &self.state,
            capabilities(self.harness).support(Feature::Resume),
        ) {
            (WorkerState::Saved(_), Support::Partial(reason)) => Some(reason),
            (WorkerState::Saved(_) | WorkerState::Running, _) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> WorkerId {
        WorkerId::parse("sched-a").unwrap()
    }

    fn session() -> SessionRef {
        SessionRef::new(PathBuf::from("/store/claude-session/sched-a"))
    }

    fn partial_reason(harness: HarnessId) -> Option<&'static str> {
        match capabilities(harness).support(Feature::Resume) {
            Support::Partial(reason) => Some(reason),
            Support::Full | Support::None => None,
        }
    }

    #[test]
    fn a_worker_id_is_letters_digits_and_dashes() {
        let parsed = WorkerId::parse("sched-a1");
        assert_eq!(parsed.as_ref().map(WorkerId::as_str), Ok("sched-a1"));
        assert_eq!(WorkerId::parse(""), Err(WorkerIdError::Empty));
        for text in ["a b", "a/b", ".sched-a", "a_b"] {
            let refused = WorkerIdError::NotDashed(text.to_string());
            assert_eq!(WorkerId::parse(text), Err(refused));
        }
    }

    #[test]
    fn a_harness_that_cannot_resume_has_no_saved_worker() {
        assert_eq!(
            Worker::saved(id(), HarnessId::ClaudeTmux, session()),
            Err("claude-tmux cannot resume a saved session.".to_string())
        );
    }

    #[test]
    fn every_other_harness_has_a_saved_worker() {
        for harness in HarnessId::ALL {
            let can_resume = capabilities(harness).support(Feature::Resume) != Support::None;
            assert_eq!(
                Worker::saved(id(), harness, session()).is_ok(),
                can_resume,
                "{harness:?}"
            );
        }
    }

    #[test]
    fn a_saved_worker_carries_the_limit_of_a_partial_resume() {
        let claude = Worker::saved(id(), HarnessId::Claude, session()).unwrap();
        let opencode = Worker::saved(id(), HarnessId::Opencode, session()).unwrap();
        assert!(partial_reason(HarnessId::Claude).is_some());
        assert_eq!(claude.resume_limit(), partial_reason(HarnessId::Claude));
        assert_eq!(opencode.resume_limit(), None);
    }

    #[test]
    fn a_running_worker_has_no_resume_limit() {
        let running = Worker::running(id(), HarnessId::Claude);
        assert_eq!(running.state(), &WorkerState::Running);
        assert_eq!(running.resume_limit(), None);
    }
}
