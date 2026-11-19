use crate::stream_processor::{BlockRenderer, OutputBlock};
use ralph_core::chunk::{ChunkType, ParsedChunk};

fn prose(content: &str) -> OutputBlock {
    OutputBlock::text(ParsedChunk {
        chunk_type: ChunkType::Prose,
        content: content.to_string(),
    })
}

#[test]
fn a_separator_before_any_counting_block_renders_nothing() {
    let mut renderer = BlockRenderer::new(None, false).unwrap();
    assert_eq!(renderer.render(&[OutputBlock::Separator]), "");
    assert!(!renderer.has_emitted_output());
}

#[test]
fn a_separator_after_a_counting_block_renders_a_newline() {
    let mut renderer = BlockRenderer::new(None, false).unwrap();
    renderer.render(&[prose("hello\n")]);
    assert_eq!(renderer.render(&[OutputBlock::Separator]), "\n");
}
