use super::ChangeFormat;

fn is_stat_summary(line: &str) -> bool {
    line.contains(" changed") && (line.contains("file changed") || line.contains("files changed"))
}

fn omitted_files(format: ChangeFormat, omitted: &str) -> usize {
    omitted
        .lines()
        .filter(|line| match format {
            ChangeFormat::Patch => line.starts_with("diff --git "),
            ChangeFormat::Files => !line.trim().is_empty(),
            ChangeFormat::Stat => !line.trim().is_empty() && !is_stat_summary(line),
        })
        .count()
}

fn cap_footer(omitted_bytes: usize, files: usize) -> String {
    format!(
        "… {omitted_bytes} bytes omitted ({files} files not shown); narrow with paths or format=stat"
    )
}

fn char_floor(text: &str, limit: usize) -> usize {
    (0..=limit.min(text.len()))
        .rev()
        .find(|index| text.is_char_boundary(*index))
        .unwrap_or(0)
}

fn kept_prefix(output: &str, budget: usize) -> usize {
    let whole_lines = output
        .split_inclusive('\n')
        .scan(0usize, |used, line| {
            *used += line.len();
            Some(*used)
        })
        .take_while(|used| *used <= budget)
        .last();
    whole_lines.unwrap_or_else(|| {
        let first_line = output.find('\n').unwrap_or(output.len());
        char_floor(output, budget.min(first_line))
    })
}

pub(super) fn cap_output(format: ChangeFormat, output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }
    let reserve = cap_footer(output.len(), omitted_files(format, output)).len() + 1;
    let keep = kept_prefix(output, max_bytes.saturating_sub(reserve));
    let (kept, omitted) = output.split_at(keep);
    let footer = cap_footer(omitted.len(), omitted_files(format, omitted));
    if kept.is_empty() {
        footer
    } else {
        format!("{}\n{footer}", kept.trim_end_matches('\n'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capped(format: ChangeFormat, output: &str, max_bytes: usize) -> String {
        cap_output(format, output, max_bytes)
    }

    #[test]
    fn cap_under_limit_passes_through() {
        let output = "M\ta.rs\nM\tb.rs\n";
        assert_eq!(capped(ChangeFormat::Files, output, output.len()), output);
    }

    #[test]
    fn cap_keeps_whole_lines_and_counts_omitted_files() {
        let output = "M\ta.rs\nM\tb.rs\nM\tc.rs\nM\td.rs\n".repeat(5);
        let body = capped(ChangeFormat::Files, &output, 120);
        assert!(body.len() <= 120, "{}", body.len());
        let (kept, footer) = body.rsplit_once('\n').unwrap();
        assert!(kept.lines().all(|line| line.starts_with("M\t")));
        assert!(output.starts_with(kept));
        let omitted = output.len() - kept.len() - 1;
        let files = output[kept.len() + 1..].lines().count();
        assert_eq!(
            footer,
            format!(
                "… {omitted} bytes omitted ({files} files not shown); narrow with paths or format=stat"
            )
        );
    }

    #[test]
    fn cap_counts_patch_files_by_diff_headers() {
        let file = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
        let output = file.repeat(6);
        let body = capped(ChangeFormat::Patch, &output, 200);
        assert!(body.len() <= 200);
        let shown = body.matches("diff --git ").count();
        let footer = body.lines().last().unwrap();
        assert!(
            footer.contains(&format!("({} files not shown)", 6 - shown)),
            "{footer}"
        );
    }

    #[test]
    fn cap_stat_footer_excludes_summary_line() {
        let output = format!(
            "{} 6 files changed, 6 insertions(+), 6 deletions(-)\n",
            " f.rs | 2 +-\n".repeat(6)
        );
        let body = capped(ChangeFormat::Stat, &output, 125);
        let footer = body.lines().last().unwrap();
        assert!(footer.contains("(3 files not shown)"), "{footer}");
        assert!(body.len() <= 125);
        assert!(body.starts_with(" f.rs | 2 +-\n"));
    }

    #[test]
    fn cap_cuts_oversized_first_line_at_char_boundary() {
        let output = format!("{}\nsecond\n", "é".repeat(100));
        let body = capped(ChangeFormat::Patch, &output, 150);
        let kept = body.lines().next().unwrap();
        assert!(!kept.is_empty());
        assert!(kept.chars().all(|c| c == 'é'));
        assert!(body.len() <= 150);
    }

    #[test]
    fn cap_tiny_limit_returns_only_the_footer() {
        let body = capped(ChangeFormat::Files, "M\ta.rs\nM\tb.rs\n", 5);
        assert!(body.starts_with("… 14 bytes omitted (2 files not shown)"));
    }
}
