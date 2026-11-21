use crate::subprocess::{
    Session, SpinnerSubprocessConfig, StreamingSubprocessResult, SubprocessError,
};
use ralph_core::cancel::CancelToken;
use ralph_core::chat_backend::{Caps, CLAUDE};
use std::time::Duration;

pub trait ChatSession: Send {
    fn turn(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
    ) -> Result<StreamingSubprocessResult, SubprocessError>;
    fn ping(
        &mut self,
        prompt: &str,
        timeout: Duration,
    ) -> Result<StreamingSubprocessResult, SubprocessError>;
    fn is_alive(&mut self) -> bool;
    fn discard(&mut self);
}

impl ChatSession for Session {
    fn turn(
        &mut self,
        prompt: &str,
        cancel: &CancelToken,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        Session::turn(self, prompt, cancel)
    }

    fn ping(
        &mut self,
        prompt: &str,
        timeout: Duration,
    ) -> Result<StreamingSubprocessResult, SubprocessError> {
        Session::ping(self, prompt, timeout)
    }

    fn is_alive(&mut self) -> bool {
        Session::is_alive(self)
    }

    fn discard(&mut self) {
        Session::discard(self)
    }
}

pub trait ChatBackend: Send + Sync {
    fn capabilities(&self) -> Caps;
    fn spawn(
        &self,
        config: &SpinnerSubprocessConfig,
    ) -> Result<Box<dyn ChatSession>, SubprocessError>;
}

pub struct ClaudeBackend;

impl ChatBackend for ClaudeBackend {
    fn capabilities(&self) -> Caps {
        CLAUDE
    }

    fn spawn(
        &self,
        config: &SpinnerSubprocessConfig,
    ) -> Result<Box<dyn ChatSession>, SubprocessError> {
        Ok(Box::new(Session::spawn(config)?))
    }
}
