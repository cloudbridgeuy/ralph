use super::*;
use crate::file_link::path_line;

const TOO_WIDE: Miss = Miss::TooWide {
    needs: 114,
    has: 46,
};

#[test]
fn diagram_name_is_sha256_hex_of_source_and_style() {
    let name = diagram_name("flowchart TD\n  A --> B", b"{}");
    let (hex, extension) = name.split_once('.').unwrap();
    assert_eq!(extension, "png");
    assert_eq!(hex.len(), 64);
    assert!(hex
        .bytes()
        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    assert_eq!(name, diagram_name("flowchart TD\n  A --> B", b"{}"));
}

#[test]
fn same_source_other_style_gets_another_name() {
    let source = "flowchart TD\n  A --> B";
    assert_ne!(
        diagram_name(source, b"{}"),
        diagram_name(source, b"{\"x\":1}")
    );
    assert_ne!(
        diagram_name(source, b"{}"),
        diagram_name("flowchart LR", b"{}")
    );
}

#[test]
fn source_and_style_bytes_cannot_trade_places() {
    assert_ne!(diagram_name("ab", b"c"), diagram_name("a", b"bc"));
}

#[test]
fn miss_note_names_needed_and_pane_columns_then_the_path() {
    let path = Path::new("/s/diagrams/abc.png");
    let note = miss_note(TOO_WIDE, Some(path));
    let (first, rest) = note.split_once('\n').unwrap();
    assert!(
        first.contains("mermaid: too big for this pane (needs 114 columns, has 46); full size:"),
        "{first:?}"
    );
    assert_eq!(rest, path_line(path));
}

#[test]
fn miss_note_says_when_the_full_size_picture_failed() {
    let note = miss_note(TOO_WIDE, None);
    assert!(note.contains("needs 114 columns, has 46"), "{note:?}");
    assert!(note.contains("the full-size picture failed"), "{note:?}");
    assert_eq!(note.matches('\n').count(), 1);
    assert!(note.ends_with('\n'));
}

#[test]
fn miss_note_without_a_text_view_still_points_at_the_file() {
    let path = Path::new("/s/diagrams/abc.png");
    let note = miss_note(Miss::NoText, Some(path));
    assert!(
        note.contains("mermaid: no text view for this diagram; full size:"),
        "{note:?}"
    );
    assert!(note.ends_with(&path_line(path)));
}
