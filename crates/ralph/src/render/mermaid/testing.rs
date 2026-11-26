use super::{mermaid_precheck, DiagramKind, Refusal};
use ralph_server::config::MermaidSection;

pub(super) fn verdict(source: &str) -> Result<DiagramKind, Refusal> {
    mermaid_precheck(source, &MermaidSection::default())
}

pub(super) fn repeated(header: &str, rows: usize, row: impl Fn(usize) -> String) -> String {
    let body: String = (0..rows).map(|i| format!("  {}\n", row(i))).collect();
    format!("{header}\n{body}")
}

pub(super) fn assert_draws(source: &str, kind: DiagramKind) {
    assert_eq!(verdict(source), Ok(kind), "{source}");
    let drawn = mermaid_text::render_with_width(source, Some(100));
    assert!(drawn.is_ok(), "{source}: {drawn:?}");
}

pub(super) fn assert_unsupported(sources: &[&str]) {
    for source in sources {
        assert_eq!(verdict(source), Err(Refusal::Unsupported), "{source}");
    }
}
