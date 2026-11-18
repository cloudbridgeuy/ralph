use super::*;

fn word(line: &str) -> PathWord {
    let found = path_word(line, line.len());
    assert!(found.is_some(), "no path word in {line:?}");
    found.unwrap()
}

fn entry(name: &str, is_dir: bool) -> DirEntry {
    DirEntry {
        name: name.to_string(),
        is_dir,
    }
}

fn values(line: &str, entries: &[DirEntry]) -> Vec<String> {
    path_completion(&word(line), entries).values
}

#[test]
fn each_prefix_kind_parses_with_its_root() {
    let cases = [
        ("see ./src/ma", PathRoot::Cwd, "./src/", "ma"),
        ("see ../a/b", PathRoot::Cwd, "../a/", "b"),
        ("see /etc/ho", PathRoot::Root, "/etc/", "ho"),
        ("see ~/Doc", PathRoot::Home, "~/", "Doc"),
        ("see $HOME/Doc", PathRoot::Home, "$HOME/", "Doc"),
        ("see .git", PathRoot::Cwd, "", ".git"),
    ];
    for (line, root, typed_dir, partial) in cases {
        let found = word(line);
        assert_eq!(found.root, root, "{line}");
        assert_eq!(found.typed_dir, typed_dir, "{line}");
        assert_eq!(found.partial, partial, "{line}");
    }
}

#[test]
fn bare_prefixes_complete_as_directories() {
    let cases = [
        ("a .", PathRoot::Cwd, "./"),
        ("a ..", PathRoot::Cwd, "../"),
        ("a ~", PathRoot::Home, "~/"),
        ("a $HOME", PathRoot::Home, "$HOME/"),
    ];
    for (line, root, typed_dir) in cases {
        let found = word(line);
        assert_eq!(found.root, root, "{line}");
        assert_eq!(found.typed_dir, typed_dir, "{line}");
        assert_eq!(found.partial, "", "{line}");
    }
}

#[test]
fn trailing_slash_lists_the_directory() {
    for line in ["a ./", "a ../", "a /", "a ~/", "a $HOME/"] {
        assert_eq!(word(line).partial, "", "{line}");
    }
}

#[test]
fn span_covers_the_word_and_the_prefix_is_kept() {
    let line = "go ~/Doc";
    let found = word(line);
    assert_eq!(found.span, 3..line.len());
    let entries = [entry("Documents", true)];
    assert_eq!(values(line, &entries), vec!["~/Documents/"]);
}

#[test]
fn partial_filters_entries() {
    let entries = [entry("main.rs", false), entry("lib.rs", false)];
    assert_eq!(values("x ./ma", &entries), vec!["./main.rs"]);
}

#[test]
fn hidden_entries_show_only_after_a_typed_dot() {
    let entries = [
        entry(".git", true),
        entry(".env", false),
        entry("src", true),
    ];
    assert_eq!(values("x ./", &entries), vec!["./src/"]);
    assert_eq!(values("x ./.", &entries), vec!["./.env", "./.git/"]);
    assert_eq!(values("x .g", &entries), vec![".git/"]);
}

#[test]
fn dirs_end_in_a_slash_and_values_are_sorted() {
    let entries = [entry("b", false), entry("c", true), entry("a", true)];
    assert_eq!(
        values("x /etc/", &entries),
        vec!["/etc/a/", "/etc/b", "/etc/c/"]
    );
}

#[test]
fn the_cap_applies_after_sorting() {
    let entries: Vec<DirEntry> = (0..MAX_SHOWN + 50)
        .rev()
        .map(|number| entry(&format!("f{number:04}"), false))
        .collect();
    let shown = values("x ./", &entries);
    assert_eq!(shown.len(), MAX_SHOWN);
    assert_eq!(shown[0], "./f0000");
    assert_eq!(shown[MAX_SHOWN - 1], format!("./f{:04}", MAX_SHOWN - 1));
}

#[test]
fn spaces_are_escaped_on_insert_and_unescaped_on_parse() {
    let entries = [entry("my notes", true), entry("my.txt", false)];
    assert_eq!(
        values("x ./my", &entries),
        vec!["./my.txt", "./my\\ notes/"]
    );
    let found = word("x ./my\\ no");
    assert_eq!(found.partial, "my no");
    assert_eq!(found.span.start, 2);
    let nested = word("x ./a\\ b/c");
    assert_eq!(nested.typed_dir, "./a\\ b/");
    assert_eq!(nested.dir(), "./a b/");
    let entries = [entry("c d", false)];
    assert_eq!(values("x ./a\\ b/c", &entries), vec!["./a\\ b/c\\ d"]);
}

#[test]
fn dir_strips_the_home_prefix() {
    assert_eq!(word("x ~/a/b").dir(), "a/");
    assert_eq!(word("x $HOME/a/b").dir(), "a/");
    assert_eq!(word("x ~/").dir(), "");
    assert_eq!(word("x /etc/ho").dir(), "/etc/");
}

#[test]
fn a_first_word_slash_command_is_not_a_path() {
    assert_eq!(path_word("/hold", 5), None);
    assert_eq!(path_word("/", 1), None);
    assert_eq!(path_word("  /hold", 7), None);
    assert!(path_word("/etc/", 5).is_some());
    assert!(path_word("/etc/ho", 7).is_some());
}

#[test]
fn a_slash_word_after_the_first_is_a_path() {
    assert_eq!(word("see /hold").root, PathRoot::Root);
    assert_eq!(word("see /hold").partial, "hold");
}

#[test]
fn other_users_and_longer_names_are_not_paths() {
    for line in [
        "a ~bob/x",
        "a ~bob",
        "a $HOMEX",
        "a $HOMEX/y",
        "a plain",
        "a src/x",
    ] {
        assert_eq!(path_word(line, line.len()), None, "{line}");
    }
}

#[test]
fn only_text_before_the_cursor_counts() {
    let line = "x ./src/main.rs tail";
    let found = path_word(line, 8);
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.span, 2..8);
    assert_eq!(found.typed_dir, "./src/");
    assert_eq!(found.partial, "");
    assert_eq!(path_word("é ./a", 1), None);
}
