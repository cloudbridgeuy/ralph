use super::*;

mod pill;
mod primitives;
mod shell;
mod spacing;
mod tree;
mod typography;

fn bytes_of(path: &str) -> &'static [u8] {
    find(path).unwrap().bytes
}

fn shell() -> String {
    String::from_utf8(bytes_of("/").to_vec()).unwrap()
}

fn script() -> String {
    String::from_utf8(bytes_of("/app.js").to_vec()).unwrap()
}

fn css() -> String {
    String::from_utf8(bytes_of("/assets/app.css").to_vec()).unwrap()
}

fn rule_body<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
    let head = format!("\n{selector} {{\n");
    let start = css.find(&head)? + head.len();
    let end = css[start..].find('}')?;
    Some(&css[start..start + end])
}

fn declarations<'a>(css: &'a str, property: &str) -> Vec<&'a str> {
    css.lines()
        .map(str::trim)
        .filter(|line| line.starts_with(property))
        .collect()
}

#[test]
fn rule_body_reads_the_block_of_an_exact_selector() {
    let css = "\nbody {\n  color: red;\n}\n\nbody-x {\n  color: blue;\n}\n";
    assert_eq!(rule_body(css, "body"), Some("  color: red;\n"));
    assert_eq!(rule_body(css, "main"), None);
}

#[test]
fn declarations_match_only_the_property_prefix() {
    let css = "a {\n  border: 0;\n  border-top: 1px;\n  outline: none;\n}\n";
    assert_eq!(
        declarations(css, "border"),
        vec!["border: 0;", "border-top: 1px;"]
    );
}

fn assert_declares(css: &str, selector: &str, declaration: &str) {
    let body = rule_body(css, selector).unwrap_or_else(|| panic!("{selector} has no rule"));
    assert!(body.contains(declaration), "{selector} lacks {declaration}");
}
