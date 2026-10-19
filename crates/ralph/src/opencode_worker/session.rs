use super::post_prompt;
use crate::acp_worker::{push_event, Events};
use crate::transport::http::HttpSse;
use crate::warn::warn;
use ralph_core::worker::{
    answer_decision, capabilities, decide, describe, parse_interrupted, permission_reply_body,
    permission_reply_path, session_missing, Capabilities, HarnessId, HeadlessWorker,
    PermissionAsked, WorkerCommand, WorkerEvent,
};
use std::sync::PoisonError;

pub(super) struct OpencodeSession {
    pub(super) id: String,
    pub(super) session_id: String,
    pub(super) api: Option<HttpSse>,
    pub(super) front: Option<PermissionAsked>,
    pub(super) caps: Capabilities,
    pub(super) events: Events,
}

impl OpencodeSession {
    pub(super) fn new(
        id: &str,
        session_id: &str,
        api: Option<HttpSse>,
        front: Option<PermissionAsked>,
        events: Events,
    ) -> Self {
        Self {
            id: id.to_string(),
            session_id: session_id.to_string(),
            api,
            front,
            caps: capabilities(HarnessId::Opencode),
            events,
        }
    }

    fn api(&self) -> Result<&HttpSse, String> {
        self.api
            .as_ref()
            .ok_or_else(|| "the opencode server is not running".to_string())
    }

    fn interrupt(&self) -> Result<bool, String> {
        let body = self
            .api()?
            .post(&format!("/api/session/{}/interrupt", self.session_id), "")?;
        parse_interrupted(&body).ok_or_else(|| format!("unreadable interrupt reply: {body}"))
    }

    fn close(&self) -> Result<(), String> {
        match self
            .api()?
            .delete(&format!("/api/session/{}", self.session_id))
        {
            Err(message) if !session_missing(&message) => {
                Err(format!("could not delete the opencode session: {message}"))
            }
            _ => Ok(()),
        }
    }
}

impl HeadlessWorker for OpencodeSession {
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    fn send(&mut self, command: WorkerCommand) -> Result<(), String> {
        let command = decide(&self.caps, command).map_err(|event| match event {
            WorkerEvent::Unsupported { reason, .. } => reason,
            other => format!("{other:?}"),
        })?;
        match command {
            WorkerCommand::Prompt(text) => post_prompt(self.api()?, &self.session_id, &text),
            WorkerCommand::Interrupt(_) => {
                let outcome = self.interrupt();
                if let Err(message) = &outcome {
                    warn(format!(
                        "could not interrupt {}: {message}",
                        self.session_id
                    ));
                }
                let id = self.id.clone();
                push_event(
                    &self.events,
                    WorkerEvent::Interrupted {
                        id,
                        outcome: outcome.ok(),
                    },
                );
                Ok(())
            }
            WorkerCommand::AnswerPermission { option_id } => {
                let (request_id, decision) = answer_decision(self.front.as_ref(), &option_id)?;
                self.api()?
                    .post_retrying(
                        &permission_reply_path(&self.session_id, &request_id),
                        &permission_reply_body(decision).to_string(),
                    )
                    .map(|_| ())
            }
            WorkerCommand::Close => {
                self.close()?;
                push_event(&self.events, WorkerEvent::Closed);
                Ok(())
            }
            WorkerCommand::Spawn(_) | WorkerCommand::Resume => Err(format!(
                "not sent: the host handles it ({})",
                describe(&self.caps, command.feature())
            )),
        }
    }

    fn poll(&mut self) -> Vec<WorkerEvent> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }
}
