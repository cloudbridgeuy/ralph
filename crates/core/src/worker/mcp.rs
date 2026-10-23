use super::{
    capabilities, decide, is_claude_model, is_cursor_model, is_model_id, is_token_model,
    model_refusal, Command, Feature, HarnessId, SpawnSpec, Support, WorkerCommand, WorkerEvent,
    CODEX_HARNESS, CURSOR_HARNESS, OPENCODE_HARNESS,
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
                    "harness": {"type": "string", "enum": HarnessId::ALL.map(HarnessId::as_str), "description": harness_description()},
                    "model": {"type": "string", "description": "claude: any model name or alias; default [claude] model in the ralph config, else claude's own default. opencode: provider/model; default [opencode] model in the ralph config, else openai/gpt-6.1-sol. cursor: any model name or alias; default [cursor] model in the ralph config, else cursor's own default. Cursor's ACP fixes the variant for each model. codex: any model name or alias; default codex's own default."}
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
                    "interrupt": {"type": "boolean", "description": "cursor, codex and opencode workers: cancel the running turn, if any, and run this message next. The result says whether a running turn was cancelled, or that the outcome is unknown if ralph chat does not confirm within 30 s. Default false."}
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

fn harness_description() -> String {
    let headless = HarnessId::ALL
        .into_iter()
        .filter(|harness| capabilities(*harness).headless())
        .map(|harness| {
            let caps = capabilities(harness);
            let partial = Feature::ALL
                .into_iter()
                .filter_map(|feature| match caps.support(feature) {
                    Support::Partial(reason) => Some(format!("{} ({reason})", feature.name())),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let unsupported = Feature::ALL
                .into_iter()
                .filter(|feature| caps.support(*feature) == Support::None)
                .map(|feature| feature.name().to_string())
                .collect::<Vec<_>>();
            let limits = [("partial", partial), ("unsupported", unsupported)]
                .into_iter()
                .filter(|(_, items)| !items.is_empty())
                .map(|(label, items)| format!("{label}: {}", items.join(", ")))
                .collect::<Vec<_>>()
                .join("; ");
            if limits.is_empty() {
                harness.as_str().to_string()
            } else {
                format!("{} ({limits})", harness.as_str())
            }
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "Agent to run. Default claude. Headless harnesses run without a tmux session and take a reply as a new prompt between turns, or as the answer to a waiting permission request: {headless}."
    )
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
}

fn harness(args: &Value) -> Result<ParsedSpawn, String> {
    let model = args.get("model").and_then(Value::as_str);
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
        (Some(CODEX_HARNESS), None) => Ok((HarnessId::Codex, None)),
        (Some(CODEX_HARNESS), Some(model)) if is_token_model(model) => {
            Ok((HarnessId::Codex, Some(model.to_string())))
        }
        (Some(CODEX_HARNESS), Some(_)) => Err("Model must be non-empty with no whitespace.".into()),
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
    Ok(ParsedSpawn { harness, model })
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
            json!({"harness": "gemini"}),
            json!({"harness": "codex", "model": "two words"}),
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
    fn harness_description_follows_capability_matrix() {
        let text = harness_description();
        let (_, list) = text.split_once("request: ").unwrap();
        let entries = list.trim_end_matches('.').split(" | ").collect::<Vec<_>>();
        let headless = HarnessId::ALL
            .into_iter()
            .filter(|harness| capabilities(*harness).headless())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), headless.len());
        for (harness, entry) in headless.into_iter().zip(entries) {
            assert!(entry.starts_with(harness.as_str()));
            for feature in Feature::ALL {
                let expected = match capabilities(harness).support(feature) {
                    Support::Full => continue,
                    Support::Partial(reason) => format!("{} ({reason})", feature.name()),
                    Support::None => feature.name().to_string(),
                };
                assert!(entry.contains(&expected), "{harness:?} {feature:?}");
            }
        }
    }

    #[test]
    fn harness_description_lists_codex_limits_as_partial() {
        let text = harness_description();
        let codex = text
            .split(" | ")
            .find(|entry| entry.starts_with("codex"))
            .unwrap();
        assert!(codex.contains("interrupt (outcome unknown after 30 s)"));
        assert!(codex.contains("resume (needs saved history"));
        assert!(codex.contains("close (global trust entry"));
        assert!(codex.contains("heartbeat (app-server frames"));
        assert!(!codex.contains("unsupported"));
        assert!(!text.contains("partial or unsupported"));
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
