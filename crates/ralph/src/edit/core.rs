use super::types::{EditError, EditMessage, IterationUpdate};
use crate::iteration::IterationLog;

/// Convert iteration logs into a flat list of edit messages.
///
/// For each log, emits a user message if a prompt exists, followed by an
/// assistant message if a response exists. Logs with neither are skipped.
pub fn iterations_to_messages(logs: &[IterationLog]) -> Vec<EditMessage> {
    logs.iter()
        .flat_map(|log| {
            let user = log.prompt.as_ref().map(|p| EditMessage {
                role: "user".to_string(),
                content: p.clone(),
            });
            let assistant = log.response.as_ref().map(|r| EditMessage {
                role: "assistant".to_string(),
                content: r.clone(),
            });
            user.into_iter().chain(assistant)
        })
        .collect()
}

/// Generate a TOML representation of the conversation for editing.
///
/// Produces a string with header comments describing the editing instructions,
/// followed by `[[messages]]` blocks for each message.
pub fn messages_to_edit_toml(msgs: &[EditMessage], slug: &str) -> String {
    let mut out = String::new();

    // Header comments
    out.push_str(&format!("# Session: {slug}\n"));
    out.push_str("# Editing conversation history. Each [[messages]] block is one turn.\n");
    out.push_str("# - Edit `content` to modify a message\n");
    out.push_str("# - Delete a [[messages]] block to remove it from history\n");
    out.push_str("# - Add a new [[messages]] block to insert a message\n");
    out.push_str("# Roles: \"user\" or \"assistant\"\n");

    for msg in msgs {
        out.push('\n');
        out.push_str("[[messages]]\n");
        out.push_str(&format!("role = \"{}\"\n", msg.role));
        // Escape any `"""` sequences in the content so they don't terminate the
        // TOML multi-line basic string. In TOML, `\"` is a valid escape inside
        // `"""…"""`, so replacing the third consecutive quote keeps the output valid.
        let escaped = msg.content.replace("\"\"\"", "\"\"\\\"");
        out.push_str(&format!("content = \"\"\"\n{escaped}\n\"\"\"\n"));
    }

    out
}

/// Parse TOML content back into edit messages.
///
/// Extracts the `[[messages]]` array and validates each entry has a valid
/// `role` ("user" or "assistant") and a `content` string.
pub fn parse_edit_toml(content: &str) -> Result<Vec<EditMessage>, EditError> {
    let parsed: toml::Value =
        toml::from_str(content).map_err(|e| EditError::ParseEditToml(e.to_string()))?;

    let empty = vec![];
    let messages = parsed
        .get("messages")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);

    let mut result = Vec::with_capacity(messages.len());

    for (i, msg) in messages.iter().enumerate() {
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| EditError::ParseEditToml(format!("Message {i} missing 'role' field")))?
            .to_string();

        if role != "user" && role != "assistant" {
            return Err(EditError::InvalidRole { role, index: i });
        }

        let content = msg.get("content").and_then(|v| v.as_str()).ok_or_else(|| {
            EditError::ParseEditToml(format!("Message {i} missing 'content' field"))
        })?;

        // TOML multi-line strings include a trailing newline — trim it.
        let content = content.strip_suffix('\n').unwrap_or(content).to_string();

        result.push(EditMessage { role, content });
    }

    Ok(result)
}

/// Re-pair a flat list of messages into iteration-style (prompt, response) tuples.
///
/// Walks through messages sequentially:
/// - A `user` message followed by an `assistant` message forms one iteration with both
/// - A `user` message followed by another `user` (or end) forms one iteration with prompt only
/// - An `assistant` message without a preceding `user` forms one iteration with response only
pub fn pair_messages_to_iterations(
    messages: &[EditMessage],
) -> Vec<(Option<String>, Option<String>)> {
    let mut iterations = Vec::new();
    let mut i = 0;

    while i < messages.len() {
        if messages[i].role == "user" {
            let prompt = Some(messages[i].content.clone());
            // Check if the next message is an assistant response
            if i + 1 < messages.len() && messages[i + 1].role == "assistant" {
                let response = Some(messages[i + 1].content.clone());
                iterations.push((prompt, response));
                i += 2;
            } else {
                iterations.push((prompt, None));
                i += 1;
            }
        } else {
            // Assistant message without preceding user
            let response = Some(messages[i].content.clone());
            iterations.push((None, response));
            i += 1;
        }
    }

    iterations
}

/// Compare new iterations against existing logs to produce update operations.
///
/// For each position up to min(old, new): Rewrite if prompt or response changed.
/// For positions only in old: Delete.
/// For positions only in new: Create.
/// Sequences are 1-indexed (matching existing convention).
pub fn plan_iteration_updates(
    logs: &[IterationLog],
    new_iterations: &[(Option<String>, Option<String>)],
) -> Vec<IterationUpdate> {
    let mut updates = Vec::new();
    let min_len = logs.len().min(new_iterations.len());

    for i in 0..min_len {
        let (new_prompt, new_response) = &new_iterations[i];
        let old_prompt = &logs[i].prompt;
        let old_response = &logs[i].response;

        if new_prompt != old_prompt || new_response != old_response {
            updates.push(IterationUpdate::Rewrite {
                sequence: logs[i].sequence,
                prompt: new_prompt.clone(),
                response: new_response.clone(),
            });
        }
    }

    // Deletions: iterations that exist in old but not in new
    for log in logs.iter().skip(min_len) {
        updates.push(IterationUpdate::Delete {
            sequence: log.sequence,
        });
    }

    // Additions: iterations that exist in new but not in old
    for (i, (prompt, response)) in new_iterations.iter().enumerate().skip(min_len) {
        updates.push(IterationUpdate::Create {
            sequence: (i + 1) as u32,
            prompt: prompt.clone(),
            response: response.clone(),
        });
    }

    updates
}
