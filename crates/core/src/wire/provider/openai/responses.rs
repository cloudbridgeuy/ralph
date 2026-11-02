use serde_json::{json, Value};

use super::{
    build_usage, count, error_message, items, reported, segment, text, usage_object, Counts,
};
use crate::wire::provider::{PromptSegment, ResponseUsage, SegmentSource};
use crate::wire::sse::SseEvent;
use crate::wire::usage::Usage;

fn is_system_item(item: &Value) -> bool {
    matches!(text(item, "role", ""), "system" | "developer")
}

fn item_segments(source: SegmentSource, index: usize, item: &Value) -> Vec<PromptSegment> {
    let Some(role) = item.get("role").and_then(Value::as_str) else {
        return vec![segment(
            source,
            format!("input[{index}] {}", text(item, "type", "?")),
            item,
        )];
    };
    match item.get("content") {
        Some(content @ Value::String(_)) => vec![segment(
            source,
            format!("input[{index}] {role}/text"),
            &json!({"type": "text", "text": content}),
        )],
        Some(Value::Array(parts)) => parts
            .iter()
            .enumerate()
            .map(|(j, part)| {
                let kind = text(part, "type", "?");
                segment(source, format!("input[{index}].{j} {role}/{kind}"), part)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn input_segments(body: &Value) -> Vec<PromptSegment> {
    match body.get("input") {
        Some(input @ Value::String(_)) => vec![segment(
            SegmentSource::Messages,
            "input user/text".to_string(),
            &json!({"type": "text", "text": input}),
        )],
        _ => {
            let leading = items(body, "input")
                .take_while(|item| is_system_item(item))
                .count();
            items(body, "input")
                .enumerate()
                .flat_map(|(i, item)| {
                    let source = if i < leading {
                        SegmentSource::System
                    } else {
                        SegmentSource::Messages
                    };
                    item_segments(source, i, item)
                })
                .collect()
        }
    }
}

pub(super) fn segments(body: &Value) -> Vec<PromptSegment> {
    let tools = items(body, "tools").enumerate().map(|(i, tool)| {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| text(tool, "type", "?"));
        segment(SegmentSource::Tools, format!("tools[{i}] {name}"), tool)
    });
    let instructions = body
        .get("instructions")
        .filter(|instructions| instructions.is_string())
        .map(|instructions| {
            segment(
                SegmentSource::System,
                "instructions".to_string(),
                &json!({"type": "text", "text": instructions}),
            )
        });
    tools
        .chain(instructions)
        .chain(input_segments(body))
        .collect()
}

fn map_usage(raw: &Value) -> Usage {
    let input = raw.get("input_tokens_details");
    let output = raw.get("output_tokens_details");
    let detail = |details: Option<&Value>, key: &str| details.and_then(|d| count(d, key));
    build_usage(Counts {
        input_total: count(raw, "input_tokens").unwrap_or(0),
        cached: detail(input, "cached_tokens"),
        written: detail(input, "cache_write_tokens"),
        output: count(raw, "output_tokens").unwrap_or(0),
        reasoning: detail(output, "reasoning_tokens"),
        other: Vec::new(),
    })
}

pub(super) fn is_response(value: &Value) -> bool {
    text(value, "object", "") == "response"
}

fn event_type(data: &Value) -> &str {
    text(data, "type", "")
}

pub(super) fn is_stream(events: &[SseEvent]) -> bool {
    events
        .iter()
        .filter_map(|event| serde_json::from_str::<Value>(&event.data).ok())
        .any(|data| event_type(&data).starts_with("response.") || event_type(&data) == "error")
}

fn response_error(response: &Value) -> Option<String> {
    match text(response, "status", "") {
        "incomplete" => {
            let reason = response
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("incomplete: {reason}"))
        }
        "failed" => error_message(response).or_else(|| Some("failed".to_string())),
        _ => None,
    }
}

pub(super) fn json_usage(response: &Value) -> ResponseUsage {
    reported(usage_object(response), response_error(response), map_usage)
}

fn error_event_message(data: &Value) -> String {
    data.get("message")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map_or_else(|| data.to_string(), String::from)
}

pub(super) fn event_usage(events: &[SseEvent]) -> ResponseUsage {
    let mut terminal = None;
    let mut error = None;
    for data in events
        .iter()
        .filter_map(|event| serde_json::from_str::<Value>(&event.data).ok())
    {
        match event_type(&data) {
            "response.completed" | "response.incomplete" | "response.failed" => {
                terminal = data.get("response").map(json_usage);
            }
            "error" if error.is_none() => error = Some(error_event_message(&data)),
            _ => {}
        }
    }
    match terminal {
        Some(result) => ResponseUsage {
            error: result.error.clone().or(error),
            ..result
        },
        None => reported(None, error, map_usage),
    }
}
