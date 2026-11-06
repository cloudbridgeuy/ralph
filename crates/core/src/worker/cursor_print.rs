use super::transcript_usage::ModelUsage;
use super::{TurnEnd, SERVER_NAME};
use crate::chat_memory::{worker_usage_record, CallSite, LedgerRow};
use crate::stream::result_is_error;
use crate::wire::usage::{CacheTier, CacheWrite, Usage};
use crate::worker_costs::long_prompt_call;
use serde_json::{json, Value};
use std::path::Path;

const HARNESS: &str = "cursor";

#[cfg(test)]
mod fixtures_tests;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintEvent {
    Init(String),
    Activity,
    Result {
        end: TurnEnd,
        usage: Option<ModelUsage>,
    },
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    Ended,
    Interrupted,
    ProcessGone,
}

#[derive(Clone, Copy)]
pub struct PrintArgs<'a> {
    pub workspace: &'a Path,
    pub model: &'a str,
    pub resume: Option<&'a str>,
}

pub fn print_args(options: &PrintArgs) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--trust",
        "--approve-mcps",
        "--workspace",
    ]
    .map(String::from)
    .into();
    args.push(options.workspace.display().to_string());
    args.extend(["--model".to_string(), options.model.to_string()]);
    if let Some(chat) = options.resume {
        args.extend(["--resume".to_string(), chat.to_string()]);
    }
    args.push("--force".to_string());
    args
}

pub fn decode_print_line(line: &str, model: &str) -> PrintEvent {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return PrintEvent::Ignored;
    };
    match (value["type"].as_str(), value["subtype"].as_str()) {
        (Some("system"), Some("init")) => value["session_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map_or(PrintEvent::Ignored, |id| PrintEvent::Init(id.to_string())),
        (Some("assistant"), _) | (Some("tool_call"), Some("started" | "completed")) => {
            PrintEvent::Activity
        }
        (Some("result"), _) => PrintEvent::Result {
            end: turn_end(line, &value),
            usage: map_print_usage(&value["usage"], model),
        },
        _ => PrintEvent::Ignored,
    }
}

fn turn_end(line: &str, result: &Value) -> TurnEnd {
    if result_is_error(line) != Some(true) {
        return TurnEnd::Succeeded;
    }
    let message = result["result"]
        .as_str()
        .or(result["subtype"].as_str())
        .unwrap_or("cursor turn failed");
    TurnEnd::Failed {
        message: message.to_string(),
        transient: false,
    }
}

pub fn exit_outcome(saw_result: bool, sigint_sent: bool) -> ExitOutcome {
    match (saw_result, sigint_sent) {
        (true, _) => ExitOutcome::Ended,
        (false, true) => ExitOutcome::Interrupted,
        (false, false) => ExitOutcome::ProcessGone,
    }
}

pub fn map_print_usage(usage: &Value, model: &str) -> Option<ModelUsage> {
    let count = |key: &str| usage[key].as_u64();
    let (input, output) = (count("inputTokens")?, count("outputTokens")?);
    let (read, write) = (count("cacheReadTokens")?, count("cacheWriteTokens")?);
    let usage = Usage {
        input_uncached: input,
        cache_read: read,
        cache_writes: if write > 0 {
            vec![CacheWrite {
                tier: CacheTier::Unspecified,
                tokens: write,
            }]
        } else {
            Vec::new()
        },
        output,
        reasoning: None,
        other: Vec::new(),
    };
    Some(ModelUsage {
        model: model.to_string(),
        long_prompt: long_prompt_call(model, &usage),
        usage,
    })
}

pub fn print_usage_record(
    worker_id: &str,
    turn_id: Option<u64>,
    at: String,
    chat: Option<String>,
    usage: &ModelUsage,
) -> LedgerRow {
    let kind = CallSite::Worker {
        worker_id: worker_id.to_string(),
        turn_id,
    };
    worker_usage_record(kind, at, chat, HARNESS, usage)
}

pub fn turn_end_for(outcome: ExitOutcome, result: Option<TurnEnd>) -> TurnEnd {
    match (outcome, result) {
        (ExitOutcome::Interrupted, _) => TurnEnd::Interrupted,
        (_, Some(end)) => end,
        (_, None) => TurnEnd::Failed {
            message: "the cursor agent exited without a result".to_string(),
            transient: false,
        },
    }
}

pub fn next_chat(current: Option<&str>, event: &PrintEvent) -> Option<String> {
    match (current, event) {
        (_, PrintEvent::Init(chat)) => Some(chat.clone()),
        (current, _) => current.map(str::to_string),
    }
}

pub fn print_model(model: Option<&str>) -> &str {
    model.unwrap_or("auto")
}

pub fn cursor_mcp_json(existing: Option<&str>, server: &Value) -> String {
    let mut config = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if !config["mcpServers"].is_object() {
        config["mcpServers"] = json!({});
    }
    config["mcpServers"][SERVER_NAME] = server.clone();
    config.to_string()
}
