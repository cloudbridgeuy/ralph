use crate::chat_cell::{trim_output, CellLimits};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeScope {
    Worker(String),
    WorkerTurn { worker: String, turn: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeFormat {
    Stat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRequest {
    pub scope: ChangeScope,
    pub format: ChangeFormat,
    pub paths: Vec<String>,
    pub max_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRange {
    pub from: String,
    pub to: Option<String>,
}

pub fn diff_args(request: &ChangeRequest, range: &SnapshotRange) -> Vec<String> {
    let mut args: Vec<String> = ["git", "--no-pager", "diff", "--no-color", "--no-ext-diff"]
        .iter()
        .map(|word| word.to_string())
        .collect();
    args.push(format_flag(request.format).to_string());
    args.push(range.from.clone());
    args.extend(range.to.iter().cloned());
    args.push("--".to_string());
    args.extend(request.paths.iter().cloned());
    args
}

fn format_flag(format: ChangeFormat) -> &'static str {
    match format {
        ChangeFormat::Stat => "--stat",
    }
}

pub fn scope_label(scope: &ChangeScope) -> String {
    match scope {
        ChangeScope::Worker(worker) => format!("changed in worker {worker} since spawn"),
        ChangeScope::WorkerTurn { worker, turn } => {
            format!("changed in worker {worker} during turn {turn}")
        }
    }
}

pub fn render_changes(request: &ChangeRequest, output: &str) -> String {
    let body = if output.trim().is_empty() {
        "no changes".to_string()
    } else {
        trim_output(
            output,
            CellLimits {
                lines: None,
                bytes: request.max_bytes,
            },
        )
    };
    format!("{}\n{}", scope_label(&request.scope), body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(paths: &[&str], max_bytes: Option<usize>) -> ChangeRequest {
        ChangeRequest {
            scope: ChangeScope::Worker("GUZ-1".to_string()),
            format: ChangeFormat::Stat,
            paths: paths.iter().map(|path| path.to_string()).collect(),
            max_bytes,
        }
    }

    fn range(to: Option<&str>) -> SnapshotRange {
        SnapshotRange {
            from: "aaa".to_string(),
            to: to.map(str::to_string),
        }
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    #[test]
    fn diff_args_stat_with_both_ids() {
        let args = diff_args(&request(&[], None), &range(Some("bbb")));
        assert_eq!(
            args,
            strings(&[
                "git",
                "--no-pager",
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--stat",
                "aaa",
                "bbb",
                "--"
            ])
        );
    }

    #[test]
    fn diff_args_paths_follow_separator() {
        let args = diff_args(&request(&["src/a.rs", "-weird"], None), &range(Some("bbb")));
        assert_eq!(
            args[6..],
            strings(&["aaa", "bbb", "--", "src/a.rs", "-weird"])[..]
        );
    }

    #[test]
    fn diff_args_empty_paths_still_end_with_separator() {
        let args = diff_args(&request(&[], None), &range(None));
        assert_eq!(args.last().map(String::as_str), Some("--"));
    }

    #[test]
    fn scope_label_worker_since_spawn() {
        assert_eq!(
            scope_label(&ChangeScope::Worker("GUZ-1".to_string())),
            "changed in worker GUZ-1 since spawn"
        );
    }

    #[test]
    fn scope_label_worker_turn() {
        let scope = ChangeScope::WorkerTurn {
            worker: "GUZ-1".to_string(),
            turn: 2,
        };
        assert_eq!(scope_label(&scope), "changed in worker GUZ-1 during turn 2");
    }

    #[test]
    fn render_empty_output_says_no_changes() {
        let req = request(&[], None);
        assert_eq!(
            render_changes(&req, ""),
            "changed in worker GUZ-1 since spawn\nno changes"
        );
        assert_eq!(
            render_changes(&req, " \n\t\n"),
            "changed in worker GUZ-1 since spawn\nno changes"
        );
    }

    #[test]
    fn render_under_cap_passes_through() {
        let req = request(&[], Some(100));
        assert_eq!(
            render_changes(&req, "M\ta.rs\n"),
            "changed in worker GUZ-1 since spawn\nM\ta.rs\n"
        );
    }

    #[test]
    fn render_over_cap_keeps_head_and_tail() {
        let req = request(&[], Some(20));
        let output = format!("HEAD{}TAIL", "x".repeat(200));
        let rendered = render_changes(&req, &output);
        assert!(rendered.contains("bytes trimmed"));
        assert!(rendered.contains("HEAD"));
        assert!(rendered.ends_with("TAIL"));
        assert!(rendered.len() < output.len());
    }

    #[test]
    fn render_without_cap_keeps_everything() {
        let req = request(&[], None);
        let output = "y".repeat(5000);
        let rendered = render_changes(&req, &output);
        assert!(!rendered.contains("bytes trimmed"));
        assert!(rendered.ends_with(&output));
    }

    #[test]
    fn render_label_is_never_trimmed() {
        let req = request(&[], Some(2));
        let rendered = render_changes(&req, &"z".repeat(100));
        assert!(rendered.starts_with("changed in worker GUZ-1 since spawn\n"));
    }
}
