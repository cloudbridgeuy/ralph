use super::*;

#[test]
fn bare_view_is_usage_and_the_rest_of_the_line_is_the_path() {
    assert_eq!(parse_view_args(""), ViewArgs::Usage);
    assert_eq!(parse_view_args("   "), ViewArgs::Usage);
    assert_eq!(
        parse_view_args(" docs/my notes.md "),
        ViewArgs::File("docs/my notes.md".to_string())
    );
}

#[test]
fn a_readable_markdown_file_is_not_refused() {
    for name in ["a.md", "b.MD", "c.markdown", "d.Markdown"] {
        let kind = FileKind::File { bytes: 10 };
        assert_eq!(view_refusal(Path::new(name), kind), None, "{name}");
    }
    let limit = FileKind::File {
        bytes: MAX_VIEW_BYTES,
    };
    assert_eq!(view_refusal(Path::new("a.md"), limit), None);
}

#[test]
fn missing_and_directory_paths_say_so_first() {
    assert_eq!(
        view_refusal(Path::new("gone.md"), FileKind::Missing).as_deref(),
        Some("/view: no such file: gone.md")
    );
    assert_eq!(
        view_refusal(Path::new("docs"), FileKind::Dir).as_deref(),
        Some("/view: docs is a directory")
    );
}

#[test]
fn other_files_and_oversized_files_are_refused() {
    let small = FileKind::File { bytes: 1 };
    for name in ["notes.txt", "README", "a.md.bak"] {
        assert_eq!(
            view_refusal(Path::new(name), small).as_deref(),
            Some(format!("/view shows .md and .markdown files: {name}").as_str()),
        );
    }
    let big = FileKind::File {
        bytes: MAX_VIEW_BYTES + 1,
    };
    assert_eq!(
        view_refusal(Path::new("big.md"), big).as_deref(),
        Some("/view: big.md is 2097153 bytes; the limit is 2097152")
    );
}

#[test]
fn relative_image_paths_resolve_from_the_file_directory() {
    let text = "intro ![logo](img/logo.png) and ![abs](/tmp/a.png)\n![home](~/b.png)\n";
    assert_eq!(
        rebase_images(text, Path::new("/docs")),
        "intro ![logo](/docs/img/logo.png) and ![abs](/tmp/a.png)\n![home](~/b.png)\n"
    );
}

#[test]
fn images_in_code_and_remote_images_stay_as_written() {
    let text = "`![a](x.png)` ![r](https://e.com/r.png)\n```md\n![b](y.png)\n```\nend\n";
    assert_eq!(rebase_images(text, Path::new("/docs")), text);
}

#[test]
fn text_without_images_is_unchanged() {
    let text = "# Title\n\nplain *text*\n";
    assert_eq!(rebase_images(text, Path::new("/docs")), text);
}
