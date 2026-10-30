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
        WorkerCommand::AnswerPermission(answer) => {
            Some(AcpCall::PermissionResponse(answer.option_id))
        }
        WorkerCommand::Spawn(_) | WorkerCommand::Resume | WorkerCommand::Close => None,
    }
}

pub fn decode_update(update: &Value, at: u64) -> Vec<WorkerEvent> {
    let mut events = vec![WorkerEvent::Activity { at }];
    if is_report_tool_call(update) {
        events.push(WorkerEvent::Reported);
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::PermissionAnswer;
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
            encode(WorkerCommand::AnswerPermission(PermissionAnswer {
                request_id: "1".into(),
                option_id: "allow-once".into()
            })),
            Some(AcpCall::PermissionResponse("allow-once".into()))
        );
        assert_eq!(encode(WorkerCommand::Close), None);
    }

    #[test]
    fn every_update_decodes_to_activity_and_the_report_tool_adds_reported() {
        let report = json!({"sessionUpdate": "tool_call_update", "title": "ralph-worker: report"});
        let chunk = json!({"sessionUpdate": "agent_message_chunk"});
        assert_eq!(
            decode_update(&report, 7),
            vec![WorkerEvent::Activity { at: 7 }, WorkerEvent::Reported]
        );
        assert_eq!(
            decode_update(&chunk, 7),
            vec![WorkerEvent::Activity { at: 7 }]
        );
    }
}
