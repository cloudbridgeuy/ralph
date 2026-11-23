use std::path::PathBuf;

pub fn long_text() -> String {
    "very-long-unbroken-token-".repeat(12)
}

pub fn hostile_text() -> &'static str {
    "<script>\"x\" & 'y'</script>"
}

pub fn assert_golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/render")
        .join(format!("{name}.html"));
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, format!("{actual}\n"))
            .unwrap_or_else(|error| panic!("golden {} is unwritable ({error})", path.display()));
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("golden {} is unreadable ({error})", path.display()));
    let expected = expected.strip_suffix('\n').unwrap_or(&expected);
    assert!(
        expected == actual,
        "golden {} differs:\n- {expected}\n+ {actual}",
        path.display()
    );
}
