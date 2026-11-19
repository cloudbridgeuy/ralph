use crate::worker::WorkerCommand;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::PermissionAnswer;

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
}
