use super::{Reach, Store, RECALL_CHUNK_ENTRIES};
use ralph_core::chat_memory::{
    cover_candidates, cover_size, found_metrics, read_metrics, recall_found, search_found,
    zoom_metrics, Archive, ByteSource, CallMetrics, Found, NodeSource, Tool,
};
use ralph_core::chat_thread::{label_item, label_lines};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    pub facts: CallMetrics,
}

fn lookup_answer<B: ByteSource>(
    archive: &mut Archive<B>,
    found: Found,
    lines: impl FnOnce(String) -> String,
) -> Answer {
    let total = archive.count();
    let cover = found.top.and_then(|top| {
        let nodes = archive.tree.nodes(&cover_candidates(top, total)).ok()?;
        Some(cover_size(&nodes, top, total))
    });
    Answer {
        facts: found_metrics(&found, cover),
        text: lines(found.text),
    }
}

impl Store {
    pub fn call(&self, tool: &Tool, reach: Reach) -> Result<Answer, String> {
        let item = |text: &String| match reach.label {
            Some(name) => label_item(text, name),
            None => text.clone(),
        };
        let lines = |text: String| match reach.label {
            Some(name) => label_lines(&text, name),
            None => text,
        };
        let result = self.with_archive(|archive| {
            if let Some(end) = reach.limit {
                archive.limit_to(end);
            }
            Ok(match tool {
                Tool::Zoom(node) => archive.zoom(*node)?.map(|items| Answer {
                    text: items.iter().map(item).collect::<Vec<_>>().join("\n"),
                    facts: zoom_metrics(*node),
                }),
                Tool::Read(request) => archive.read(request)?.map(|mut page| {
                    page.blocks = page.blocks.iter().map(item).collect();
                    Answer {
                        text: page.render(),
                        facts: read_metrics(request),
                    }
                }),
                Tool::Recall(pattern) => {
                    let mut entries = Vec::new();
                    archive.scan_entries(RECALL_CHUNK_ENTRIES, |_, entry| {
                        entries.push(entry.clone())
                    })?;
                    recall_found(&entries, pattern)
                        .map(|found| lookup_answer(archive, found, lines))
                }
                Tool::Search(request) => {
                    let request = super::super::tag_store::with_tags(&self.dir, request)?;
                    search_found(archive, &request)?
                        .map(|found| lookup_answer(archive, found, lines))
                }
            })
        });
        result.unwrap_or_else(|e| Err(format!("Could not read chat memory: {e}")))
    }
}
