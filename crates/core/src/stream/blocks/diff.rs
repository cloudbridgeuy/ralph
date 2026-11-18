use similar::{ChangeTag, TextDiff};

use super::EditSnapshot;

pub fn edit_diff(snapshot: &EditSnapshot, after: &str) -> Option<String> {
    let before = snapshot.content.as_deref().unwrap_or("");
    if before == after {
        return None;
    }
    Some(unified_diff(before, after, &snapshot.file_path))
}

fn unified_diff(before: &str, after: &str, label: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut output = format!("--- a/{label}\n+++ b/{label}\n");
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&format!("{}\n", hunk.header()));
        for change in hunk.iter_changes() {
            output.push_str(match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            });
            output.push_str(change.value());
            if !change.value().ends_with('\n') {
                output.push('\n');
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_unified_diff() {
        let unified = unified_diff(
            "line1\nline2\nline3\n",
            "line1\nmodified\nline3\n",
            "test.txt",
        );
        assert!(unified.contains("--- a/test.txt"));
        assert!(unified.contains("+++ b/test.txt"));
        assert!(unified.contains("-line2"));
        assert!(unified.contains("+modified"));
    }

    #[test]
    fn test_generate_unified_diff_addition() {
        let unified = unified_diff("line1\n", "line1\nline2\n", "test.txt");
        assert!(unified.contains("+line2"));
        assert!(!unified.contains("-line1"));
    }

    #[test]
    fn test_generate_unified_diff_deletion() {
        let unified = unified_diff("line1\nline2\n", "line1\n", "test.txt");
        assert!(unified.contains("-line2"));
        assert!(!unified.contains("+line2"));
    }

    #[test]
    fn unified_diff_breaks_line_after_hunk_header() {
        let out = unified_diff("x = 1\n", "x = 2\n", "a.rs");
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.contains(&"@@ -1 +1 @@"));
        assert!(!lines
            .iter()
            .any(|l| l.starts_with("@@") && !l.ends_with("@@")));
    }
}
