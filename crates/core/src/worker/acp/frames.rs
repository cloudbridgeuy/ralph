use super::is_report_tool_call;
use crate::worker::{WorkerCommand, WorkerEvent};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpCall {
    Prompt(String),
    Cancel,
    PermissionResponse(String),
}

pub fn encode(command: WorkerCommand) -> Option<AcpCall> {
    match command {
        WorkerCommand::Prompt(text) => Some(AcpCall::Prompt(text)),
        WorkerCommand::Interrupt(_) => Some(AcpCall::Cancel),
        WorkerCommand::AnswerPermission { option_id } => {
            Some(AcpCall::PermissionResponse(option_id))
        }
        WorkerCommand::Spawn(_) | WorkerCommand::Resume | WorkerCommand::Close => None,
    }
}

pub fn decode_update(update: &Value) -> Option<WorkerEvent> {
    is_report_tool_call(update).then_some(WorkerEvent::Reported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn commands_encode_to_calls_and_lifecycle_commands_do_not() {
        assert_eq!(
            encode(WorkerCommand::Prompt("hi".into())),
            Some(AcpCall::Prompt("hi".into()))
        );
        assert_eq!(
            encode(WorkerCommand::Interrupt("x".into())),
            Some(AcpCall::Cancel)
        );
        assert_eq!(
            encode(WorkerCommand::AnswerPermission {
                option_id: "allow-once".into()
            }),
            Some(AcpCall::PermissionResponse("allow-once".into()))
        );
        assert_eq!(encode(WorkerCommand::Close), None);
    }

    #[test]
    fn only_the_report_tool_update_decodes_to_reported() {
        let report = json!({"sessionUpdate": "tool_call_update", "title": "ralph-worker: report"});
        let chunk = json!({"sessionUpdate": "agent_message_chunk"});
        assert_eq!(decode_update(&report), Some(WorkerEvent::Reported));
        assert_eq!(decode_update(&chunk), None);
    }
}
