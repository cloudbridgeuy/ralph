use serde_json::{json, Value};

use super::{
    build_usage, count, error_message, items, reported, segment, text, usage_object, Counts,
};
use crate::wire::provider::{PromptSegment, ResponseUsage, SegmentSource};
use crate::wire::sse::SseEvent;
use crate::wire::usage::Usage;

fn is_system(message: &Value) -> bool {
    matches!(text(message, "role", ""), "system" | "developer")
}

fn with_tool_call_id(message: &Value, block: &Value) -> Value {
    let mut tagged = block.clone();
    if let (Some(object), Some(id)) = (tagged.as_object_mut(), message.get("tool_call_id")) {
        object.insert("tool_call_id".to_string(), id.clone());
    }
    tagged
}

fn function_name(tool: &Value) -> &str {
    tool.pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| text(tool, "name", "?"))
}

fn message_segments(source: SegmentSource, index: usize, message: &Value) -> Vec<PromptSegment> {
    let role = text(message, "role", "?");
    let system = source == SegmentSource::System;
    let tagged = |block: &Value| with_tool_call_id(message, block);
    let mut segments = match message.get("content") {
        Some(content @ Value::String(_)) => {
            let label = if system {
                format!("system[{index}]")
            } else {
                format!("msg[{index}] {role}/text")
            };
            vec![segment(
                source,
                label,
                &tagged(&json!({"type": "text", "text": content})),
            )]
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .enumerate()
            .map(|(j, part)| {
                let kind = text(part, "type", "?");
                let label = if system {
                    format!("system[{index}].{j} {kind}")
                } else {
                    format!("msg[{index}].{j} {role}/{kind}")
                };
                segment(source, label, &tagged(part))
            })
            .collect(),
        _ => Vec::new(),
    };
    if let Some(calls) = message.get("tool_calls").filter(|calls| calls.is_array()) {
        segments.push(segment(
            source,
            format!("msg[{index}] {role}/tool_calls"),
            calls,
        ));
    }
    segments
}

pub(super) fn segments(body: &Value) -> Vec<PromptSegment> {
    let tools = items(body, "tools").enumerate().map(|(i, tool)| {
        segment(
            SegmentSource::Tools,
            format!("tools[{i}] {}", function_name(tool)),
            tool,
        )
    });
    let leading = items(body, "messages").take_while(|m| is_system(m)).count();
    let messages = items(body, "messages")
        .enumerate()
        .flat_map(move |(i, message)| {
            let source = if i < leading {
                SegmentSource::System
            } else {
                SegmentSource::Messages
            };
            message_segments(source, i, message)
        });
    tools.chain(messages).collect()
}

fn map_usage(raw: &Value) -> Usage {
    let prompt = raw.get("prompt_tokens_details");
    let completion = raw.get("completion_tokens_details");
    let detail = |details: Option<&Value>, key: &str| details.and_then(|d| count(d, key));
    build_usage(Counts {
        input_total: count(raw, "prompt_tokens").unwrap_or(0),
        cached: detail(prompt, "cached_tokens"),
        written: detail(prompt, "cache_write_tokens"),
        output: count(raw, "completion_tokens").unwrap_or(0),
        reasoning: detail(completion, "reasoning_tokens"),
        other: vec![
            ("prompt_audio_tokens", detail(prompt, "audio_tokens")),
            (
                "completion_audio_tokens",
                detail(completion, "audio_tokens"),
            ),
            (
                "accepted_prediction_tokens",
                detail(completion, "accepted_prediction_tokens"),
            ),
            (
                "rejected_prediction_tokens",
                detail(completion, "rejected_prediction_tokens"),
            ),
        ],
    })
}

pub(super) fn event_usage(events: &[SseEvent]) -> ResponseUsage {
    let mut raw = None;
    let mut error = None;
    for data in events
        .iter()
        .filter_map(|event| serde_json::from_str::<Value>(&event.data).ok())
    {
        if error.is_none() {
            error = error_message(&data);
        }
        if let Some(usage) = usage_object(&data) {
            raw = Some(usage);
        }
    }
    reported(raw, error, map_usage)
}

pub(super) fn json_usage(value: &Value) -> ResponseUsage {
    reported(usage_object(value), error_message(value), map_usage)
}
