use super::*;

#[test]
fn syntax_notices_prints_path_and_text_per_licence() {
    let out = syntax_notices(&[("syntaxes/Rust/LICENSE", "MIT text\n")]);
    assert_eq!(
        out,
        format!("{HEADER}syntaxes/Rust/LICENSE\n\nMIT text\n\n")
    );
}

#[test]
fn syntax_notices_keeps_licence_order() {
    let out = syntax_notices(&[("syntaxes/Zig/LICENSE", "Z"), ("syntaxes/Ada/LICENSE", "A")]);
    assert_eq!(
        out,
        format!("{HEADER}syntaxes/Zig/LICENSE\n\nZ\n\nsyntaxes/Ada/LICENSE\n\nA\n\n")
    );
}

#[test]
fn font_notices_prints_name_and_text_per_font() {
    let out = font_notices(&[("Geist Mono", "OFL text\n")]);
    assert_eq!(out, format!("{FONT_HEADER}Geist Mono\n\nOFL text\n\n"));
}

#[test]
fn font_notices_keeps_font_order_and_names_the_web() {
    let out = font_notices(&[("B", "b"), ("A", "a")]);
    assert_eq!(out, format!("{FONT_HEADER}B\n\nb\n\nA\n\na\n\n"));
    assert!(FONT_HEADER.contains("ralph web"));
}
