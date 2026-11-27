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

fn entry(name: &str, is_dir: bool) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        is_dir,
    }
}

#[test]
fn path_query_splits_the_typed_path_at_the_last_slash() {
    let line = "/view docs/my no";
    assert_eq!(
        view_path_query(line, line.len()),
        Some(PathQuery {
            span: 6..16,
            dir: "docs/".to_string(),
            prefix: "my no".to_string(),
        })
    );
    assert_eq!(
        view_path_query("/view ", 6),
        Some(PathQuery {
            span: 6..6,
            dir: String::new(),
            prefix: String::new(),
        })
    );
    assert_eq!(
        view_path_query("  /view ~/a", 11).map(|query| query.dir),
        Some("~/".to_string())
    );
    assert_eq!(view_path_query("/view", 5), None);
    assert_eq!(view_path_query("/viewer a", 9), None);
    assert_eq!(view_path_query("/hold a", 7), None);
}

#[test]
fn path_completion_offers_dirs_and_markdown_files_that_match() {
    let query = view_path_query("/view docs/R", 12).unwrap();
    let entries = [
        entry("README.md", false),
        entry("Rules.MARKDOWN", false),
        entry("Run.rs", false),
        entry("Recipes", true),
        entry("notes.md", false),
        entry(".Rhidden.md", false),
    ];
    let completion = view_path_completion(&query, &entries);
    assert_eq!(completion.span, 6..12);
    assert_eq!(
        completion.values,
        ["docs/README.md", "docs/Recipes/", "docs/Rules.MARKDOWN"]
    );
}

#[test]
fn hidden_entries_show_only_for_a_dot_prefix() {
    let entries = [
        entry(".notes.md", false),
        entry(".git", true),
        entry("a.md", false),
    ];
    let all = view_path_completion(&view_path_query("/view ", 6).unwrap(), &entries);
    assert_eq!(all.values, ["a.md"]);
    let dotted = view_path_completion(&view_path_query("/view .", 7).unwrap(), &entries);
    assert_eq!(dotted.values, [".git/", ".notes.md"]);
}

fn pager(program: &str, args: &[&str]) -> ViewPager {
    ViewPager::Command(PagerCommand {
        program: program.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
    })
}

#[test]
fn view_pager_defaults_to_less_raw() {
    assert_eq!(resolve_view_pager(None), (pager("less", &["-r"]), None));
}

#[test]
fn view_pager_none_prints_direct() {
    assert_eq!(
        resolve_view_pager(Some(" none ")),
        (ViewPager::Direct, None)
    );
}

#[test]
fn view_pager_splits_the_program_from_its_arguments() {
    assert_eq!(
        resolve_view_pager(Some(" bat  --paging=always -p ")),
        (pager("bat", &["--paging=always", "-p"]), None)
    );
}

#[test]
fn an_empty_view_pager_warns_and_uses_the_default() {
    for empty in ["", "  "] {
        let (resolved, warning) = resolve_view_pager(Some(empty));
        assert_eq!(resolved, pager("less", &["-r"]), "{empty:?}");
        assert!(warning.is_some_and(|w| w.contains("view_pager")));
    }
}

#[test]
fn output_that_is_not_a_terminal_is_printed_without_pager_or_wait() {
    for resolved in [ViewPager::Direct, pager("less", &["-r"])] {
        for stdin_tty in [true, false] {
            assert_eq!(show_plan(&resolved, stdin_tty, Stdout::NotTty), Show::Print);
        }
    }
}

#[test]
fn direct_output_waits_only_when_keys_can_be_read() {
    let direct = ViewPager::Direct;
    assert_eq!(show_plan(&direct, true, Stdout::Tty), Show::PrintAndWait);
    assert_eq!(show_plan(&direct, false, Stdout::Tty), Show::Print);
    let less = pager("less", &["-r"]);
    for stdin_tty in [true, false] {
        let plan = show_plan(&less, stdin_tty, Stdout::Tty);
        assert!(
            matches!(plan, Show::Page(command) if command.program == "less"),
            "{plan:?}"
        );
    }
}

#[test]
fn q_and_ctrl_c_quit_the_wait() {
    for key in [&b"q"[..], b"qq", b"\x03"] {
        assert!(quits(key), "{key:?}");
    }
    for key in [&b""[..], b"x", b"\r", b"\x1b[B", b"\x1bq"] {
        assert!(!quits(key), "{key:?}");
    }
}
