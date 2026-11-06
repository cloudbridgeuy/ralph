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
