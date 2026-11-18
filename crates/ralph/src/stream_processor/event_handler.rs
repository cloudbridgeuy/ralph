use std::collections::HashMap;

use ralph_core::chat_send_it::is_step_result_tool;
use ralph_core::chunk::{
    split_lines_preserve_trailing, ChunkType, ParsedChunk, StreamingChunkBuffer,
};
use ralph_core::directive::split_text_around_directives;
use ralph_core::stream::blocks::{
    build_result_block, build_tool_invocation_block, capture_edit_snapshot,
    capture_notebook_snapshot, capture_write_snapshot, Files, OutputBlock, Snapshot,
    StreamProcessorResult, VerboseToolsConfig,
};
use ralph_core::stream::{
    extract_costs_from_events_or_default, extract_metadata_from_events_or_default,
    parse_stream_line, ParsedLine, StreamEvent, ToolInvocation,
};

pub type Visible<'a> = &'a mut dyn FnMut(&OutputBlock) -> bool;

pub struct StreamCollector {
    events: Vec<StreamEvent>,
    chunk_buffer: StreamingChunkBuffer,
    current_message_id: Option<String>,
    collected_chunks: Vec<ParsedChunk>,
    parse_errors: Vec<(String, String)>,
    has_emitted_output: bool,
    response_count: usize,
    pub(super) pending_invocations: HashMap<String, ToolInvocation>,
    pub(super) pending_snapshots: HashMap<String, Snapshot>,
    output_blocks: Vec<OutputBlock>,
    show_tool_invocations: bool,
    verbose_tools_config: VerboseToolsConfig,
}

fn record(out: &mut Vec<OutputBlock>, blocks: Vec<OutputBlock>, visible: Visible) -> bool {
    let mut any_visible = false;
    for block in blocks {
        any_visible |= visible(&block);
        out.push(block);
    }
    any_visible
}

fn capture_snapshot(invocation: &ToolInvocation, files: &dyn Files) -> Option<Snapshot> {
    match invocation.name.as_str() {
        "Edit" => capture_edit_snapshot(invocation, files).map(Snapshot::Edit),
        "Write" => capture_write_snapshot(invocation, files).map(Snapshot::Write),
        "NotebookEdit" => capture_notebook_snapshot(invocation, files).map(Snapshot::Notebook),
        _ => None,
    }
}

impl StreamCollector {
    pub fn new(show_tool_invocations: bool, verbose_tools_config: VerboseToolsConfig) -> Self {
        Self {
            events: Vec::new(),
            chunk_buffer: StreamingChunkBuffer::with_prose_threshold(usize::MAX),
            current_message_id: None,
            collected_chunks: Vec::new(),
            parse_errors: Vec::new(),
            has_emitted_output: false,
            response_count: 0,
            pending_invocations: HashMap::new(),
            pending_snapshots: HashMap::new(),
            output_blocks: Vec::new(),
            show_tool_invocations,
            verbose_tools_config,
        }
    }

    pub fn process_line(
        &mut self,
        line: &str,
        files: &dyn Files,
        visible: Visible,
    ) -> Vec<OutputBlock> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        match parse_stream_line(trimmed) {
            ParsedLine::Event(event) => self.handle_event(event, files, visible),
            ParsedLine::Empty => Vec::new(),
            ParsedLine::Error { line: _, error } => {
                self.parse_errors.push((trimmed.to_string(), error));
                Vec::new()
            }
        }
    }

    pub fn finish(mut self) -> (Vec<OutputBlock>, StreamProcessorResult) {
        let flushed = self.flush_pending_chunks();
        let result = StreamProcessorResult {
            chunks: self.collected_chunks,
            metadata: extract_metadata_from_events_or_default(&self.events),
            costs: extract_costs_from_events_or_default(&self.events),
            output_blocks: self.output_blocks,
            final_output: None,
        };
        (flushed, result)
    }

    pub fn parse_errors(&self) -> &[(String, String)] {
        &self.parse_errors
    }

    pub fn response_count(&self) -> usize {
        self.response_count
    }

    pub fn has_emitted_output(&self) -> bool {
        self.has_emitted_output
    }

    pub fn output_blocks(&self) -> &[OutputBlock] {
        &self.output_blocks
    }

    pub fn is_showing_tool_invocations(&self) -> bool {
        self.show_tool_invocations
    }

    pub fn verbose_tools_config(&self) -> &VerboseToolsConfig {
        &self.verbose_tools_config
    }

    fn handle_event(
        &mut self,
        event: StreamEvent,
        files: &dyn Files,
        visible: Visible,
    ) -> Vec<OutputBlock> {
        let mut out = Vec::new();
        let mut any_visible = false;

        match &event {
            StreamEvent::Assistant(assistant_event) => {
                let new_message_id = assistant_event.message.id.clone();
                let is_new_response =
                    new_message_id != self.current_message_id && self.current_message_id.is_some();

                if is_new_response {
                    let flushed = self.flush_pending_chunks();
                    any_visible |= record(&mut out, flushed, visible);
                    if self.has_emitted_output || any_visible {
                        let separator = vec![self.emit(OutputBlock::separator())];
                        any_visible |= record(&mut out, separator, visible);
                    }
                    self.response_count += 1;
                } else if self.current_message_id.is_none() {
                    self.response_count = 1;
                }
                self.current_message_id = new_message_id;

                let tool_invocations = assistant_event.extract_tool_invocations();
                if !tool_invocations.is_empty() {
                    let flushed = self.flush_pending_chunks();
                    any_visible |= record(&mut out, flushed, visible);
                }

                for invocation in &tool_invocations {
                    if !self.show_tool_invocations {
                        continue;
                    }
                    self.pending_invocations
                        .insert(invocation.id.clone(), invocation.clone());
                    if let Some(snapshot) = capture_snapshot(invocation, files) {
                        self.pending_snapshots
                            .insert(invocation.id.clone(), snapshot);
                    }
                    if is_step_result_tool(&invocation.name) {
                        continue;
                    }
                    let block = build_tool_invocation_block(invocation, &self.verbose_tools_config);
                    let emitted = vec![self.emit(block)];
                    any_visible |= record(&mut out, emitted, visible);
                }

                let text = assistant_event.extract_text();
                if !text.is_empty() {
                    let blocks = self.collect_text(&text);
                    any_visible |= record(&mut out, blocks, visible);
                }
            }
            StreamEvent::User(user_event) => {
                if self.show_tool_invocations {
                    for result in &user_event.message.content {
                        let tool_use_id = result.tool_use_id.as_ref();
                        let invocation =
                            tool_use_id.and_then(|id| self.pending_invocations.remove(id));
                        let snapshot = tool_use_id.and_then(|id| self.pending_snapshots.remove(id));

                        if !result.is_error
                            && invocation
                                .as_ref()
                                .is_some_and(|inv| is_step_result_tool(&inv.name))
                        {
                            continue;
                        }

                        let block = build_result_block(
                            &self.verbose_tools_config,
                            result,
                            invocation.as_ref(),
                            snapshot,
                            files,
                        );
                        let emitted = vec![self.emit(block)];
                        any_visible |= record(&mut out, emitted, visible);
                    }
                }
            }
            StreamEvent::System(_) | StreamEvent::Result(_) => {}
        }

        self.events.push(event);
        self.has_emitted_output |= any_visible;
        out
    }

    fn emit(&mut self, block: OutputBlock) -> OutputBlock {
        self.output_blocks.push(block.clone());
        block
    }

    fn emit_chunks(&mut self, chunks: Vec<ParsedChunk>) -> Vec<OutputBlock> {
        let mut blocks = Vec::new();
        for chunk in chunks {
            let sub_chunks = match &chunk.chunk_type {
                ChunkType::Prose => split_text_around_directives(&chunk.content),
                _ => vec![chunk],
            };
            for sub_chunk in sub_chunks {
                self.collected_chunks.push(sub_chunk.clone());
                blocks.push(self.emit(OutputBlock::text(sub_chunk)));
            }
        }
        blocks
    }

    fn collect_text(&mut self, text: &str) -> Vec<OutputBlock> {
        let mut blocks = Vec::new();
        for line in split_lines_preserve_trailing(text) {
            let chunks = self.chunk_buffer.process_line(line);
            blocks.extend(self.emit_chunks(chunks));
        }
        blocks
    }

    fn flush_pending_chunks(&mut self) -> Vec<OutputBlock> {
        let chunks = self.chunk_buffer.finish();
        self.emit_chunks(chunks)
    }
}
