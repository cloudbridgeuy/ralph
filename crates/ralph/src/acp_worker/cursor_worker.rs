use super::permissions::Permissions;
use super::print::PrintWorker;
use super::AcpWorker;
use ralph_core::worker::{HeadlessWorker, WorkerCommand, CURSOR_AGENT};
use std::sync::Arc;
use std::time::Instant;

pub(super) enum CursorWorker {
    Acp(Box<AcpWorker>),
    Print(PrintWorker),
}

impl CursorWorker {
    pub(super) fn reap_exit(&mut self) {
        if let Self::Acp(worker) = self {
            worker.reap_exit();
        }
    }

    pub(super) fn apply_events(&mut self) {
        match self {
            Self::Acp(worker) => worker.apply_events(),
            Self::Print(worker) => worker.apply_events(),
        }
    }

    pub(super) fn begin_stop(&mut self) {
        match self {
            Self::Acp(worker) => worker.begin_stop(),
            Self::Print(worker) => worker.begin_stop(),
        }
    }

    pub(super) fn finish_stop(&mut self, deadline: Instant) {
        match self {
            Self::Acp(worker) => worker.finish_stop(deadline),
            Self::Print(worker) => worker.terminate(),
        }
    }

    pub(super) fn permissions(&self) -> Option<Arc<Permissions>> {
        match self {
            Self::Acp(worker) => Some(Arc::clone(&worker.permissions)),
            Self::Print(_) => None,
        }
    }

    pub(super) fn interrupt(&self, text: &str, entry: &str) {
        match self {
            Self::Acp(worker) => worker.interrupt(text, entry),
            Self::Print(worker) => worker.interrupt(text.to_string(), entry.to_string()),
        }
    }

    pub(super) fn send(&mut self, command: WorkerCommand) -> Result<(), String> {
        match (self, command) {
            (Self::Acp(worker), command) => worker.send(command),
            (Self::Print(worker), WorkerCommand::Prompt(text)) => {
                worker.accept(text);
                Ok(())
            }
            (Self::Print(worker), WorkerCommand::Close) => {
                worker.terminate();
                worker.closed();
                Ok(())
            }
            (Self::Print(_), _) => Err(format!(
                "{} -p workers run with --force and ask no permission",
                CURSOR_AGENT.command
            )),
        }
    }
}
