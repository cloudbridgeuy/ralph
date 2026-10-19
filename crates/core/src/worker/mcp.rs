use super::{
    capabilities, decide, Command, HarnessId, SpawnSpec, WorkerCommand, WorkerEvent,
    CURSOR_HARNESS, DEFAULT_OPENCODE_MODEL, OPENCODE_HARNESS,
};
use crate::mcp::string_arg;
use crate::watch_request::parse_watch_args;
use serde_json::{json, Value};

pub fn command_definitions() -> [Value; 4] {
    [
        json!({
            "name": "spawn_worker",
            "description": "Start a worker agent on a task in its own git worktree and tmux session. Only when the person asked for it in their latest message. When the worker needs a pasted image, copy its [Image #N: <path>] reference verbatim into task.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "Short id such as a ticket key: letters, digits and dashes."},
                    "task": {"type": "string", "description": "What the worker should do, in the person's terms. Keep [Image #N: <path>] references verbatim."},
                    "harness": {"type": "string", "enum": HarnessId::ALL.map(HarnessId::as_str), "description": "Agent to run. Default claude. cursor runs headless without a tmux session; a reply to it is delivered as a new prompt between its turns, or answers a waiting permission request. It resumes after a chat restart."},
                    "model": {"type": "string", "description": "opencode only: provider/model. Default openai/gpt-6.1-sol."}
                },
                "required": ["id", "task"]
            }
        }),
        json!({
            "name": "reply",
            "description": "Send a message to a running worker. Only with what the person asked in their latest message, or, when a worker asks, with an answer the person already settled in the chat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The worker id."},
                    "text": {"type": "string", "description": "The message for the worker. Keep [Image #N: <path>] references verbatim when it needs the image."},
                    "interrupt": {"type": "boolean", "description": "cursor workers only: cancel the running turn, if any, and run this message next. The result says whether a running turn was cancelled, or that the outcome is unknown if ralph chat does not confirm within 30 s. Default false."}
                },
                "required": ["id", "text"]
            }
        }),
        json!({
            "name": "close_worker",
            "description": "Close a worker of this chat: stop its tmux session and opencode session, remove its worktree and delete its worker/<id> branch. Refuses while the branch has commits not in main or the worktree has uncommitted changes, unless abandon is true, and refuses a worker of another chat unless force is true. Only when the person asked for it in their latest message.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The worker id."},
                    "abandon": {"type": "boolean", "description": "Drop unmerged commits and uncommitted changes. Only when the person said so. Default false."},
                    "force": {"type": "boolean", "description": "Close even when the session is tagged with another chat's store or is untagged. Only when the person said so. Default false."}
                },
                "required": ["id"]
            }
        }),
        crate::watch_request::watch_definition(),
    ]
}

pub fn parse_command(name: &str, args: &Value) -> Option<Result<Command, String>> {
    let parsed = match name {
        "spawn_worker" => string_arg(args, "task").and_then(|task| {
            let id = worker_id(args)?;
            let (harness, model) = harness(args)?;
            Ok(Command::Spawn {
                id,
                task,
                harness,
                model,
            })
        }),
        "reply" => string_arg(args, "text").and_then(|text| {
            let id = worker_id(args)?;
            let interrupt = bool_flag(args, "interrupt")?;
            Ok(Command::Reply {
                id,
                text,
                interrupt,
            })
        }),
        "close_worker" => worker_id(args).and_then(|id| {
            let flag = |name: &str| bool_flag(args, name);
            Ok(Command::Close {
                id,
                abandon: flag("abandon")?,
                force: flag("force")?,
            })
        }),
        "watch" => parse_watch_args(args).map(Command::Watch),
        _ => return None,
    };
    Some(parsed)
}

fn bool_flag(args: &Value, name: &str) -> Result<bool, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{name} must be true or false.")),
    }
}

fn worker_id(args: &Value) -> Result<String, String> {
    let id = string_arg(args, "id")?;
    if id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(id)
    } else {
        Err(format!(
            "Worker id '{id}' may only use letters, digits and dashes."
        ))
    }
}

fn harness(args: &Value) -> Result<(HarnessId, Option<String>), String> {
    let model = args.get("model").and_then(Value::as_str);
    match (args.get("harness").and_then(Value::as_str), model) {
        (None | Some("claude"), None) => Ok((HarnessId::Claude, None)),
        (None | Some("claude"), Some(_)) => {
            Err("A model can only be set for the opencode harness.".to_string())
        }
        (Some(OPENCODE_HARNESS), model) => {
            let model = model.unwrap_or(DEFAULT_OPENCODE_MODEL);
            match model.split_once('/') {
                Some((provider, name)) if !provider.is_empty() && !name.is_empty() => {
                    Ok((HarnessId::Opencode, Some(model.to_string())))
                }
                _ => Err(format!("Model '{model}' must look like provider/model.")),
            }
        }
        (Some(CURSOR_HARNESS), None) => Ok((HarnessId::Cursor, None)),
        (Some(CURSOR_HARNESS), Some(model)) => {
            let spec = SpawnSpec {
                model: Some(model.to_string()),
            };
            match decide(&capabilities(HarnessId::Cursor), WorkerCommand::Spawn(spec)) {
                Err(WorkerEvent::Unsupported { reason, .. }) => Err(reason),
                Err(other) => Err(format!("{other:?}")),
                Ok(_) => Ok((HarnessId::Cursor, Some(model.to_string()))),
            }
        }
        (Some(other), _) => Err(format!(
            "Unknown harness '{other}'. Use claude, opencode or cursor."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{describe, Feature};

    #[test]
    fn parse_command_reads_spawn_and_reply() {
        assert_eq!(
            parse_command("spawn_worker", &json!({"id": "GUZ-1", "task": "fix it"})),
            Some(Ok(Command::Spawn {
                id: "GUZ-1".into(),
                task: "fix it".into(),
                harness: HarnessId::Claude,
                model: None
            }))
        );
        assert_eq!(
            parse_command("reply", &json!({"id": "GUZ-1", "text": "merge"})),
            Some(Ok(Command::Reply {
                id: "GUZ-1".into(),
                text: "merge".into(),
                interrupt: false
            }))
        );
        assert_eq!(
            parse_command(
                "reply",
                &json!({"id": "GUZ-1", "text": "stop", "interrupt": true})
            ),
            Some(Ok(Command::Reply {
                id: "GUZ-1".into(),
                text: "stop".into(),
                interrupt: true
            }))
        );
        assert_eq!(parse_command("zoom", &json!({})), None);
    }

    #[test]
    fn parse_command_reads_close_with_optional_abandon() {
        assert_eq!(
            parse_command("close_worker", &json!({"id": "GUZ-1"})),
            Some(Ok(Command::Close {
                id: "GUZ-1".into(),
                abandon: false,
                force: false
            }))
        );
        assert_eq!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "abandon": true})),
            Some(Ok(Command::Close {
                id: "GUZ-1".into(),
                abandon: true,
                force: false
            }))
        );
        assert_eq!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "force": true})),
            Some(Ok(Command::Close {
                id: "GUZ-1".into(),
                abandon: false,
                force: true
            }))
        );
        assert!(matches!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "force": 1})),
            Some(Err(_))
        ));
        assert!(matches!(
            parse_command("close_worker", &json!({"id": "GUZ-1", "abandon": "yes"})),
            Some(Err(_))
        ));
        assert!(matches!(
            parse_command("close_worker", &json!({"id": "../x"})),
            Some(Err(_))
        ));
    }

    fn spawn_harness(args: Value) -> Result<(HarnessId, Option<String>), String> {
        let mut args = args;
        args["id"] = json!("A");
        args["task"] = json!("t");
        match parse_command("spawn_worker", &args) {
            Some(Ok(Command::Spawn { harness, model, .. })) => Ok((harness, model)),
            Some(Ok(other)) => panic!("unexpected {other:?}"),
            Some(Err(e)) => Err(e),
            None => panic!("not a worker command"),
        }
    }

    #[test]
    fn spawn_harness_defaults_to_claude_and_opencode_to_sol() {
        assert_eq!(spawn_harness(json!({})), Ok((HarnessId::Claude, None)));
        assert_eq!(
            spawn_harness(json!({"harness": "claude"})),
            Ok((HarnessId::Claude, None))
        );
        assert_eq!(
            spawn_harness(json!({"harness": "opencode"})),
            Ok((HarnessId::Opencode, Some(DEFAULT_OPENCODE_MODEL.into())))
        );
        assert_eq!(
            spawn_harness(json!({"harness": "opencode", "model": "openai/gpt-5.6-luna"})),
            Ok((HarnessId::Opencode, Some("openai/gpt-5.6-luna".into())))
        );
    }

    #[test]
    fn spawn_harness_rejects_bad_choices() {
        for args in [
            json!({"harness": "codex"}),
            json!({"model": "openai/gpt-6.1-sol"}),
            json!({"harness": "opencode", "model": "gpt-6.1-sol"}),
            json!({"harness": "opencode", "model": "openai/"}),
            json!({"harness": "opencode", "model": "/sol"}),
        ] {
            assert!(spawn_harness(args.clone()).is_err(), "{args}");
        }
    }

    #[test]
    fn spawn_harness_rejects_cursor_model_through_capabilities() {
        let result = spawn_harness(json!({"harness": "cursor", "model": "x/y"}));
        assert_eq!(
            result,
            Err(describe(&capabilities(HarnessId::Cursor), Feature::Model))
        );
        assert_eq!(result, Err("cursor model: not supported".to_string()));
    }

    #[test]
    fn parse_command_rejects_ids_unsafe_for_paths_and_tmux() {
        for id in ["../x", "a b", "a:b", "a/b"] {
            assert!(matches!(
                parse_command("reply", &json!({"id": id, "text": "t"})),
                Some(Err(_))
            ));
        }
    }
}
