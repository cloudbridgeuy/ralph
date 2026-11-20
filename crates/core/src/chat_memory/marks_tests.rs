use super::fixtures::{entry, n};
use super::tree::{entry_line, leaf_line};
use super::{
    mark_of, read_range, recall_found, role_label, zoom, Entry, Flags, Marks, Mode, RangeRequest,
    Read, Role, Tree, Window,
};

fn flags(rule: bool, decision: bool, approval: bool) -> Flags {
    Flags {
        rule,
        decision,
        approval,
    }
}

fn entries() -> Vec<Entry> {
    vec![entry(0, 5), entry(1, 5), entry(2, 5)]
}

fn read_text(entries: &[Entry], marks: &Marks) -> String {
    let request = RangeRequest {
        from: 0,
        to: 2,
        mode: Mode::Raw,
        budget: 4096,
        cursor: None,
    };
    let window = Window::full(entries).with_marks(marks);
    match read_range(&window, &Tree::new(), &request).unwrap() {
        Read::Page(page) => page.render(),
        Read::More => panic!("a full window never needs more"),
    }
}

#[test]
fn mark_lists_only_true_labels_in_fixed_order() {
    assert_eq!(
        mark_of(&flags(true, true, true)).as_deref(),
        Some("[rule,approval,decision]")
    );
    assert_eq!(
        mark_of(&flags(false, true, false)).as_deref(),
        Some("[decision]")
    );
    assert_eq!(
        mark_of(&flags(true, false, true)).as_deref(),
        Some("[rule,approval]")
    );
    assert_eq!(
        mark_of(&flags(false, true, true)).as_deref(),
        Some("[approval,decision]")
    );
}

#[test]
fn no_true_label_means_no_mark() {
    assert_eq!(mark_of(&flags(false, false, false)), None);
}

#[test]
fn role_label_appends_the_mark() {
    assert_eq!(role_label(Role::User, None), "user");
    assert_eq!(
        role_label(Role::User, Some(&flags(false, false, false))),
        "user"
    );
    assert_eq!(
        role_label(Role::User, Some(&flags(true, false, false))),
        "user [rule]"
    );
}

#[test]
fn entry_line_marks_the_role() {
    let e = entry(0, 5);
    assert_eq!(
        entry_line(0, &e, "body", Some(&flags(true, true, false))),
        "#0 user [rule,decision] 2026-01-01T00:00:00Z body"
    );
    assert_eq!(
        entry_line(0, &e, "body", None),
        "#0 user 2026-01-01T00:00:00Z body"
    );
}

#[test]
fn leaf_line_marks_the_role() {
    let e = entry(1, 5);
    assert_eq!(
        leaf_line(1, &e, &Tree::new(), Some(&flags(false, false, true))).unwrap(),
        format!("#1 assistant [approval] 2026-01-01T00:00:01Z {}", e.text)
    );
}

#[test]
fn zoom_marks_tagged_leaves_only() {
    let entries = entries();
    let marks = Marks::from([(0, flags(true, false, false))]);
    let window = Window::full(&entries).with_marks(&marks);
    let lines = zoom(&window, &Tree::new(), n(0, 2)).unwrap();
    assert_eq!(
        lines[0],
        format!("#0 user [rule] 2026-01-01T00:00:00Z {}", entries[0].text)
    );
    assert_eq!(
        lines[1],
        format!("#1 assistant 2026-01-01T00:00:01Z {}", entries[1].text)
    );
    let single = zoom(&window, &Tree::new(), n(0, 1)).unwrap();
    assert_eq!(
        single[0],
        format!("#0 user [rule] 2026-01-01T00:00:00Z {}", entries[0].text)
    );
}

#[test]
fn zoom_leaves_summary_lines_unmarked() {
    let entries = entries();
    let marks = Marks::from([(0, flags(true, true, true)), (1, flags(true, true, true))]);
    let mut tree = Tree::new();
    tree.insert(n(0, 2), "summary of both".into());
    let window = Window::full(&entries).with_marks(&marks);
    let lines = zoom(&window, &tree, n(0, 4)).unwrap();
    assert_eq!(lines[0], "#0-1 summary of both");
}

#[test]
fn untagged_zoom_is_byte_identical() {
    let entries = entries();
    let none = Marks::new();
    let empty = Marks::from([(2, flags(false, false, false))]);
    let plain = zoom(&Window::full(&entries), &Tree::new(), n(0, 2)).unwrap();
    let with_empty = zoom(
        &Window::full(&entries).with_marks(&empty),
        &Tree::new(),
        n(0, 2),
    )
    .unwrap();
    let with_none = zoom(
        &Window::full(&entries).with_marks(&none),
        &Tree::new(),
        n(0, 2),
    )
    .unwrap();
    assert_eq!(plain, with_empty);
    assert_eq!(plain, with_none);
    assert_eq!(
        plain[0],
        format!("#0 user 2026-01-01T00:00:00Z {}", entries[0].text)
    );
}

#[test]
fn read_header_carries_the_mark() {
    let entries = entries();
    let marks = Marks::from([
        (0, flags(true, false, false)),
        (2, flags(false, true, false)),
    ]);
    let out = read_text(&entries, &marks);
    assert!(
        out.contains("#0 · user [rule] · 2026-01-01T00:00:00Z · "),
        "{out}"
    );
    assert!(
        out.contains("#1 · assistant · 2026-01-01T00:00:01Z · "),
        "{out}"
    );
    assert!(
        out.contains("#2 · user [decision] · 2026-01-01T00:00:02Z · "),
        "{out}"
    );
}

#[test]
fn untagged_read_is_byte_identical() {
    let entries = entries();
    let empty = Marks::from([(0, flags(false, false, false))]);
    assert_eq!(
        read_text(&entries, &Marks::new()),
        read_text(&entries, &empty)
    );
}

#[test]
fn recall_line_carries_the_mark() {
    let entries = entries();
    let marks = Marks::from([(1, flags(false, false, true))]);
    let out = recall_found(&entries, "m1|m2", &marks).unwrap().text;
    assert!(
        out.contains("#1 · assistant [approval] · 2026-01-01 · "),
        "{out}"
    );
    assert!(out.contains("#2 · user · 2026-01-01 · "), "{out}");
}

#[test]
fn untagged_recall_is_byte_identical() {
    let entries = entries();
    let empty = Marks::from([(1, flags(false, false, false))]);
    assert_eq!(
        recall_found(&entries, "m", &Marks::new()).unwrap().text,
        recall_found(&entries, "m", &empty).unwrap().text
    );
}
