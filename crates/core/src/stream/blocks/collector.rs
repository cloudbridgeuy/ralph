use std::collections::HashMap;

use crate::chat_send_it::is_step_result_tool;
use crate::chunk::{split_lines_preserve_trailing, ChunkType, ParsedChunk, StreamingChunkBuffer};
use crate::directive::split_text_around_directives;
use crate::kitty::{parse_image_markers, Segment};
use crate::stream::{
    extract_costs_from_events_or_default, extract_metadata_from_events_or_default,
    parse_stream_line, ParsedLine, StreamEvent, ToolInvocation,
};

use super::block_builders::build_tool_invocation_block;
use super::output_block::{ImageFile, OutputBlock, TextBlock};
use super::result_handlers::{build_result_block, Snapshot};
use super::snapshot_capture::{
    capture_edit_snapshot, capture_notebook_snapshot, capture_write_snapshot,
};
use super::types::{StreamProcessorResult, VerboseToolsConfig};
use super::Files;

pub struct StreamCollector {
    events: Vec<StreamEvent>,
    chunk_buffer: StreamingChunkBuffer,
    current_message_id: Option<String>,
    collected_chunks: Vec<ParsedChunk>,
    parse_errors: Vec<(String, String)>,
    response_count: usize,
    pending_invocations: HashMap<String, ToolInvocation>,
    pending_snapshots: HashMap<String, Snapshot>,
    output_blocks: Vec<OutputBlock>,
    show_tool_invocations: bool,
    verbose_tools_config: VerboseToolsConfig,
}

fn image_files(chunk: &ParsedChunk, files: &dyn Files) -> Vec<ImageFile> {
    match chunk.chunk_type {
        ChunkType::Prose => parse_image_markers(&chunk.content)
            .into_iter()
            .filter_map(|segment| match segment {
                Segment::Image { path, .. } => Some(files.image(&path)),
                Segment::Text(_) => None,
            })
            .collect(),
        _ => Vec::new(),
    }
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
            response_count: 0,
            pending_invocations: HashMap::new(),
            pending_snapshots: HashMap::new(),
            output_blocks: Vec::new(),
            show_tool_invocations,
            verbose_tools_config,
        }
    }

    pub fn process_line(&mut self, line: &str, files: &dyn Files) -> Vec<OutputBlock> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }
        match parse_stream_line(trimmed) {
            ParsedLine::Event(event) => self.handle_event(event, files),
            ParsedLine::Empty => Vec::new(),
            ParsedLine::Error { line: _, error } => {
                self.parse_errors.push((trimmed.to_string(), error));
                Vec::new()
            }
        }
    }

    pub fn finish(mut self, files: &dyn Files) -> (Vec<OutputBlock>, StreamProcessorResult) {
        let flushed = self.flush_pending_chunks(files);
        let result = StreamProcessorResult {
            chunks: self.collected_chunks,
            metadata: extract_metadata_from_events_or_default(&self.events),
            costs: extract_costs_from_events_or_default(&self.events),
            output_blocks: self.output_blocks,
        };
        (flushed, result)
    }

    pub fn parse_errors(&self) -> &[(String, String)] {
        &self.parse_errors
    }

    pub fn response_count(&self) -> usize {
        self.response_count
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

    pub fn pending_invocation_count(&self) -> usize {
        self.pending_invocations.len()
    }

    pub fn pending_snapshot_count(&self) -> usize {
        self.pending_snapshots.len()
    }

    fn handle_event(&mut self, event: StreamEvent, files: &dyn Files) -> Vec<OutputBlock> {
        let mut out = Vec::new();

        match &event {
            StreamEvent::Assistant(assistant_event) => {
                let new_message_id = assistant_event.message.id.clone();
                let is_new_response =
                    new_message_id != self.current_message_id && self.current_message_id.is_some();

                if is_new_response {
                    out.extend(self.flush_pending_chunks(files));
                    out.push(self.emit(OutputBlock::separator()));
                    self.response_count += 1;
                } else if self.current_message_id.is_none() {
                    self.response_count = 1;
                }
                self.current_message_id = new_message_id;

                let tool_invocations = assistant_event.extract_tool_invocations();
                if !tool_invocations.is_empty() {
                    out.extend(self.flush_pending_chunks(files));
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
                    out.push(self.emit(block));
                }

                let text = assistant_event.extract_text();
                if !text.is_empty() {
                    out.extend(self.collect_text(&text, files));
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
                        out.push(self.emit(block));
                    }
                }
            }
            StreamEvent::System(_) | StreamEvent::Result(_) => {}
        }

        self.events.push(event);
        out
    }

    fn emit(&mut self, block: OutputBlock) -> OutputBlock {
        self.output_blocks.push(block.clone());
        block
    }

    fn emit_chunks(&mut self, chunks: Vec<ParsedChunk>, files: &dyn Files) -> Vec<OutputBlock> {
        let mut blocks = Vec::new();
        for chunk in chunks {
            let sub_chunks = match &chunk.chunk_type {
                ChunkType::Prose => split_text_around_directives(&chunk.content),
                _ => vec![chunk],
            };
            for sub_chunk in sub_chunks {
                self.collected_chunks.push(sub_chunk.clone());
                let images = image_files(&sub_chunk, files);
                blocks.push(self.emit(OutputBlock::Text(TextBlock {
                    chunk: sub_chunk,
                    images,
                })));
            }
        }
        blocks
    }

    fn collect_text(&mut self, text: &str, files: &dyn Files) -> Vec<OutputBlock> {
        let mut blocks = Vec::new();
        for line in split_lines_preserve_trailing(text) {
            let chunks = self.chunk_buffer.process_line(line);
            blocks.extend(self.emit_chunks(chunks, files));
        }
        blocks
    }

    fn flush_pending_chunks(&mut self, files: &dyn Files) -> Vec<OutputBlock> {
        let chunks = self.chunk_buffer.finish();
        self.emit_chunks(chunks, files)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use super::super::{
        Files, ImageFile, OutputBlock, StreamCollector, ToolResultVariant, VerboseToolsConfig,
    };

    struct FakeFiles(HashMap<&'static str, &'static str>);

    impl Files for FakeFiles {
        fn read_text(&self, path: &Path) -> Option<String> {
            self.0
                .get(path.to_str()?)
                .map(|content| content.to_string())
        }

        fn exists(&self, path: &Path) -> bool {
            path.to_str().is_some_and(|p| self.0.contains_key(p))
        }

        fn image(&self, path: &str) -> ImageFile {
            ImageFile {
                path: path.to_string(),
                shown: path.into(),
                bytes: self.0.get(path).map(|content| content.as_bytes().to_vec()),
            }
        }
    }

    fn feed(collector: &mut StreamCollector, files: &FakeFiles, line: &str) -> Vec<OutputBlock> {
        collector.process_line(line, files)
    }

    #[test]
    fn markdown_image_line_carries_the_file_bytes() {
        let mut collector = StreamCollector::new(true, VerboseToolsConfig::default());
        let files = FakeFiles(HashMap::from([("pic.png", "PNG")]));
        let line = r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"![alt](pic.png)\n"}]}}"#;
        let mut blocks = feed(&mut collector, &files, line);
        blocks.extend(collector.finish(&files).0);
        let images: Vec<_> = blocks
            .into_iter()
            .filter_map(|block| match block {
                OutputBlock::Text(text) => Some(text.images),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            images,
            vec![ImageFile {
                path: "pic.png".into(),
                shown: "pic.png".into(),
                bytes: Some(b"PNG".to_vec()),
            }]
        );
    }

    #[test]
    fn edit_result_carries_the_diff_between_the_fake_file_versions() {
        let mut collector = StreamCollector::new(true, VerboseToolsConfig::default());
        let before = FakeFiles(HashMap::from([("/f.rs", "one\n")]));
        let after = FakeFiles(HashMap::from([("/f.rs", "two\n")]));

        feed(
            &mut collector,
            &before,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"e1","name":"Edit","input":{"file_path":"/f.rs"}}]}}"#,
        );
        let blocks = feed(
            &mut collector,
            &after,
            r#"{"type":"user","message":{"id":"u1","content":[{"type":"tool_result","tool_use_id":"e1","content":"ok","is_error":false}]}}"#,
        );

        let [OutputBlock::ToolResult(result)] = blocks.as_slice() else {
            panic!("expected one tool result, got {blocks:?}");
        };
        let ToolResultVariant::EditDiff { diff_content, .. } = &result.variant else {
            panic!("expected an edit diff, got {:?}", result.variant);
        };
        assert!(diff_content.contains("-one\n+two\n"), "{diff_content}");
    }

    #[test]
    fn a_second_assistant_message_id_yields_a_separator() {
        let mut collector = StreamCollector::new(true, VerboseToolsConfig::default());
        let files = FakeFiles(HashMap::new());

        feed(
            &mut collector,
            &files,
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hello\n"}]}}"#,
        );
        let blocks = feed(
            &mut collector,
            &files,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"again\n"}]}}"#,
        );

        assert!(blocks.contains(&OutputBlock::Separator), "{blocks:?}");
    }
}
