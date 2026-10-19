use crate::snapshot::SnapshotBase;

pub const UPSTREAM_NOTE: &str = "note: could not separate upstream commits from these snapshots; counts may include commits pulled in by a rebase";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBase {
    Direct,
    Rebase { old: String, new: String },
    Unknown,
}

pub fn diff_base(from: Option<&SnapshotBase>, to: Option<&SnapshotBase>) -> DiffBase {
    let (Some(from), Some(to)) = (from, to) else {
        return DiffBase::Unknown;
    };
    if from.base == to.base || to.base.as_deref() == Some(to.head.as_str()) {
        return DiffBase::Direct;
    }
    match (&from.base, &to.base) {
        (Some(old), Some(new)) => DiffBase::Rebase {
            old: old.clone(),
            new: new.clone(),
        },
        _ => DiffBase::Unknown,
    }
}

pub fn merge_tree_args(old: &str, new: &str, snapshot: &str) -> Vec<String> {
    vec![
        "git".to_string(),
        "merge-tree".to_string(),
        "--write-tree".to_string(),
        format!("--merge-base={old}"),
        new.to_string(),
        snapshot.to_string(),
    ]
}

pub fn merged_tree(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
}

pub fn with_note(rendered: &str, note: Option<&str>) -> String {
    match note {
        Some(note) => format!("{rendered}\n{note}"),
        None => rendered.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(head: &str, base: Option<&str>) -> SnapshotBase {
        SnapshotBase {
            head: head.to_string(),
            base: base.map(str::to_string),
        }
    }

    #[test]
    fn missing_snapshot_is_unknown() {
        let some = snap("h", Some("b"));
        assert_eq!(diff_base(None, Some(&some)), DiffBase::Unknown);
        assert_eq!(diff_base(Some(&some), None), DiffBase::Unknown);
        assert_eq!(diff_base(None, None), DiffBase::Unknown);
    }

    #[test]
    fn same_base_is_direct() {
        assert_eq!(
            diff_base(Some(&snap("h1", Some("b"))), Some(&snap("h2", Some("b")))),
            DiffBase::Direct
        );
    }

    #[test]
    fn both_bases_missing_is_direct() {
        assert_eq!(
            diff_base(Some(&snap("h1", None)), Some(&snap("h2", None))),
            DiffBase::Direct
        );
    }

    #[test]
    fn checkout_on_the_default_branch_is_direct() {
        assert_eq!(
            diff_base(Some(&snap("h1", Some("b1"))), Some(&snap("h2", Some("h2")))),
            DiffBase::Direct
        );
    }

    #[test]
    fn moved_base_is_a_rebase() {
        assert_eq!(
            diff_base(Some(&snap("h1", Some("b1"))), Some(&snap("h2", Some("b2")))),
            DiffBase::Rebase {
                old: "b1".to_string(),
                new: "b2".to_string()
            }
        );
    }

    #[test]
    fn one_base_missing_is_unknown() {
        assert_eq!(
            diff_base(Some(&snap("h1", Some("b1"))), Some(&snap("h2", None))),
            DiffBase::Unknown
        );
        assert_eq!(
            diff_base(Some(&snap("h1", None)), Some(&snap("h2", Some("b2")))),
            DiffBase::Unknown
        );
    }

    #[test]
    fn merge_tree_args_write_a_tree_onto_the_new_base() {
        assert_eq!(
            merge_tree_args("old", "new", "snap"),
            [
                "git",
                "merge-tree",
                "--write-tree",
                "--merge-base=old",
                "new",
                "snap"
            ]
        );
    }

    #[test]
    fn merged_tree_reads_the_first_line() {
        assert_eq!(merged_tree("abc123\nextra\n"), Some("abc123".to_string()));
        assert_eq!(merged_tree(" abc \n"), Some("abc".to_string()));
    }

    #[test]
    fn merged_tree_is_none_for_empty_output() {
        assert_eq!(merged_tree(""), None);
        assert_eq!(merged_tree("  \nx"), None);
    }

    #[test]
    fn note_goes_on_its_own_line() {
        assert_eq!(with_note("body", Some("n")), "body\nn");
        assert_eq!(with_note("body", None), "body");
    }
}
