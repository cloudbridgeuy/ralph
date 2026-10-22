use super::{
    capabilities, decide, is_claude_model, is_cursor_model, is_model_id, model_refusal, Command,
    Feature, HarnessId, SpawnSpec, Support, WorkerCommand, WorkerEvent, CURSOR_HARNESS,
    OPENCODE_HARNESS,
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
                    "model": {"type": "string", "description": "claude: any model name or alias; default [claude] model in the ralph config, else claude's own default. opencode: provider/model; default [opencode] model in the ralph config, else openai/gpt-6.1-sol. cursor: any model name or alias; default [cursor] model in the ralph config, else cursor's own default."},
                    "reasoning_effort": {"type": "string", "description": "cursor only. Overrides [cursor] reasoning_effort. Applied only when that parameter is advertised for the model; otherwise the spawn fails."},
                    "fast": {"type": "boolean", "description": "cursor only. Overrides [cursor] fast. Applied only when fast is advertised for the model; otherwise the spawn fails."}
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
                    "interrupt": {"type": "boolean", "description": "cursor and opencode workers: cancel the running turn, if any, and run this message next. The result says whether a running turn was cancelled, or that the outcome is unknown if ralph chat does not confirm within 30 s. Default false."}
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
            let spawn = harness(args)?;
            Ok(Command::Spawn {
                id,
                task,
                harness: spawn.harness,
                model: spawn.model,
                reasoning_effort: spawn.reasoning_effort,
                fast: spawn.fast,
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

fn harness_choices() -> String {
    let names = HarnessId::ALL.map(HarnessId::as_str);
    match names.split_last() {
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

struct ParsedSpawn {
    harness: HarnessId,
    model: Option<String>,
    reasoning_effort: Option<String>,
    fast: Option<bool>,
}

fn optional_bool(args: &Value, name: &str) -> Result<Option<bool>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("{name} must be true or false.")),
    }
}

fn optional_token(args: &Value, name: &str) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value))
            if !value.is_empty() && !value.chars().any(char::is_whitespace) =>
        {
            Ok(Some(value.clone()))
        }
        Some(Value::String(value)) => Err(format!(
            "{name} '{value}' must be non-empty with no whitespace."
        )),
        Some(_) => Err(format!("{name} must be a string.")),
    }
}

fn harness(args: &Value) -> Result<ParsedSpawn, String> {
    let model = args.get("model").and_then(Value::as_str);
    let reasoning_effort = optional_token(args, "reasoning_effort")?;
    let fast = optional_bool(args, "fast")?;
    let (harness, model) = match (args.get("harness").and_then(Value::as_str), model) {
        (None | Some("claude"), None) => Ok((HarnessId::Claude, None)),
        (None | Some("claude"), Some(model)) => {
            if capabilities(HarnessId::Claude).support(Feature::Model) == Support::None {
                Err(model_refusal(HarnessId::Claude))
            } else if is_claude_model(model) {
                Ok((HarnessId::Claude, Some(model.to_string())))
            } else {
                Err(format!(
                    "Model '{model}' must be non-empty with no whitespace."
                ))
            }
        }
        (Some(OPENCODE_HARNESS), None) => Ok((HarnessId::Opencode, None)),
        (Some(OPENCODE_HARNESS), Some(model)) if is_model_id(model) => {
            Ok((HarnessId::Opencode, Some(model.to_string())))
        }
        (Some(OPENCODE_HARNESS), Some(model)) => {
            Err(format!("Model '{model}' must look like provider/model."))
        }
        (Some(CURSOR_HARNESS), None) => Ok((HarnessId::Cursor, None)),
        (Some(CURSOR_HARNESS), Some(model)) => {
            let spec = SpawnSpec {
                model: Some(model.to_string()),
            };
            match decide(&capabilities(HarnessId::Cursor), WorkerCommand::Spawn(spec)) {
                Err(WorkerEvent::Unsupported { reason, .. }) => Err(reason),
                Err(other) => Err(format!("{other:?}")),
                Ok(_) if is_cursor_model(model) => Ok((HarnessId::Cursor, Some(model.to_string()))),
                Ok(_) => Err(format!(
                    "Model '{model}' must be non-empty with no whitespace."
                )),
            }
        }
        (Some(other), _) => Err(format!(
            "Unknown harness '{other}'. Use {}.",
            harness_choices()
        )),
    }?;
    if harness != HarnessId::Cursor && (reasoning_effort.is_some() || fast.is_some()) {
        return Err(
            "reasoning_effort and fast are only supported for the cursor harness.".to_string(),
        );
    }
    Ok(ParsedSpawn {
        harness,
        model,
        reasoning_effort,
        fast,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_command_reads_spawn_and_reply() {
        assert_eq!(
            parse_command("spawn_worker", &json!({"id": "GUZ-1", "task": "fix it"})),
            Some(Ok(Command::Spawn {
                id: "GUZ-1".into(),
                task: "fix it".into(),
                harness: HarnessId::Claude,
                model: None,
                reasoning_effort: None,
                fast: None,
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
    fn spawn_harness_defaults_to_claude_and_opencode_without_model() {
        assert_eq!(spawn_harness(json!({})), Ok((HarnessId::Claude, None)));
        assert_eq!(
            spawn_harness(json!({"harness": "claude"})),
            Ok((HarnessId::Claude, None))
        );
        assert_eq!(
            spawn_harness(json!({"harness": "opencode"})),
            Ok((HarnessId::Opencode, None))
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
            json!({"model": "two words"}),
            json!({"harness": "claude", "model": ""}),
            json!({"harness": "opencode", "model": "gpt-6.1-sol"}),
            json!({"harness": "opencode", "model": "openai/"}),
            json!({"harness": "opencode", "model": "/sol"}),
            json!({"harness": "cursor", "model": ""}),
            json!({"harness": "cursor", "model": "two words"}),
        ] {
            assert!(spawn_harness(args.clone()).is_err(), "{args}");
        }
    }

    #[test]
    fn spawn_harness_accepts_a_cursor_model_when_the_capability_allows() {
        assert_eq!(
            spawn_harness(json!({"harness": "cursor", "model": "gpt-5"})),
            Ok((HarnessId::Cursor, Some("gpt-5".into())))
        );
    }

    #[test]
    fn spawn_harness_accepts_cursor_effort_and_fast_only_for_cursor() {
        match parse_command(
            "spawn_worker",
            &json!({"id": "A", "task": "t", "harness": "cursor", "model": "grok-4.7", "reasoning_effort": "low", "fast": false}),
        ) {
            Some(Ok(Command::Spawn {
                reasoning_effort,
                fast,
                ..
            })) => {
                assert_eq!(reasoning_effort.as_deref(), Some("low"));
                assert_eq!(fast, Some(false));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(spawn_harness(json!({"harness": "claude", "reasoning_effort": "low"})).is_err());
        assert!(
            spawn_harness(json!({"harness": "cursor", "reasoning_effort": "low effort"})).is_err()
        );
        assert!(spawn_harness(json!({"harness": "cursor", "fast": "false"})).is_err());
    }

    #[test]
    fn spawn_harness_model_follows_capability_matrix() {
        for harness in HarnessId::ALL {
            let model = "openai/gpt-6.1-sol";
            let expected = if capabilities(harness).support(Feature::Model) != Support::None {
                Ok((harness, Some(model.to_string())))
            } else {
                Err(model_refusal(harness))
            };
            assert_eq!(
                spawn_harness(json!({"harness": harness.as_str(), "model": model})),
                expected
            );
        }
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
