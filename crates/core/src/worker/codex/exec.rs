use crate::chunk::ParsedChunk;
use crate::stream::blocks::{OutputBlock, StreamProcessorResult};
use crate::stream::{OutputTokensDetails, Usage};
use serde_json::Value;

#[derive(serde::Deserialize)]
struct ExecUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    cache_write_input_tokens: Option<u64>,
    reasoning_output_tokens: Option<u64>,
}

#[derive(Default)]
pub struct ExecStream {
    result: StreamProcessorResult,
    text: Option<String>,
    completed: bool,
    error: Option<String>,
}

impl ExecStream {
    pub fn observe(&mut self, line: &str) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            self.error = Some("Invalid Codex JSONL frame".into());
            return;
        };
        match event["type"].as_str() {
            Some("thread.started") => {
                self.result.metadata.session_id = event["thread_id"].as_str().map(str::to_owned);
            }
            Some("item.completed") if event["item"]["type"] == "agent_message" => {
                self.text = event["item"]["text"].as_str().map(str::to_owned);
            }
            Some("turn.completed") => {
                self.completed = true;
                if let Ok(usage) = serde_json::from_value::<ExecUsage>(event["usage"].clone()) {
                    self.result.costs.usage = Some(Usage {
                        input_tokens: usage
                            .input_tokens
                            .saturating_sub(usage.cached_input_tokens)
                            .saturating_sub(usage.cache_write_input_tokens.unwrap_or(0)),
                        output_tokens: usage.output_tokens,
                        cache_read_input_tokens: Some(usage.cached_input_tokens),
                        cache_creation_input_tokens: usage.cache_write_input_tokens,
                        output_tokens_details: usage.reasoning_output_tokens.map(|thinking| {
                            OutputTokensDetails {
                                thinking_tokens: Some(thinking),
                            }
                        }),
                        ..Default::default()
                    });
                }
            }
            Some("turn.failed" | "error") => self.error = Some("Codex turn failed".into()),
            None => self.error = Some("Invalid Codex JSONL frame".into()),
            _ => {}
        }
    }

    pub fn finish(mut self) -> (StreamProcessorResult, Option<String>) {
        if !self.completed {
            self.error
                .get_or_insert_with(|| "Codex turn did not complete".into());
        }
        if self
            .text
            .as_deref()
            .is_none_or(|text| text.trim().is_empty())
        {
            self.error
                .get_or_insert_with(|| "Codex returned empty text".into());
        }
        if self.error.is_none() {
            let chunk = ParsedChunk::prose(self.text.unwrap_or_default());
            self.result.chunks.push(chunk.clone());
            self.result.output_blocks.push(OutputBlock::text(chunk));
        }
        (self.result, self.error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(lines: &[&str]) -> (StreamProcessorResult, Option<String>) {
        let mut stream = ExecStream::default();
        for line in lines {
            stream.observe(line);
        }
        stream.finish()
    }

    #[test]
    fn completed_stream_uses_last_message_and_normalizes_usage() {
        let (result, error) = decode(&[
            r#"{"type":"thread.started","thread_id":"thread"}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"first"}}"#,
            r#"{"type":"item.updated","item":{"type":"agent_message","text":"ignore"}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"last"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":30,"cache_write_input_tokens":5,"output_tokens":9,"reasoning_output_tokens":3}}"#,
        ]);
        assert!(error.is_none());
        assert_eq!(result.chunks[0].content, "last");
        assert_eq!(result.metadata.session_id.as_deref(), Some("thread"));
        let usage = result.costs.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.cache_read_input_tokens,
                usage.cache_creation_input_tokens,
                usage.output_tokens
            ),
            (65, Some(30), Some(5), 9)
        );
        assert_eq!(
            usage.output_tokens_details.unwrap().thinking_tokens,
            Some(3)
        );
    }

    #[test]
    fn invalid_streams_discard_text_and_return_safe_diagnostic() {
        let message =
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"secret"}}"#;
        for tail in [
            "invalid secret",
            r#"{"type":"turn.failed"}"#,
            r#"{"type":"error"}"#,
            r#"{"type":"unrelated"}"#,
        ] {
            let (result, error) = decode(&[message, tail]);
            assert!(result.chunks.is_empty());
            assert!(!error.unwrap().contains("secret"));
        }
        let (result, error) = decode(&[message, r#"{}"#, r#"{"type":"turn.completed"}"#]);
        assert!(result.chunks.is_empty());
        assert!(error.is_some());
        let (result, error) = decode(&[
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"  "}}"#,
            r#"{"type":"turn.completed"}"#,
        ]);
        assert!(result.chunks.is_empty());
        assert!(error.is_some());
    }
}
