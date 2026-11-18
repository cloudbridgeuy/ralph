use crate::chat::bounded::stdout_within;
use crate::chat::sent::SentStyle;
use ralph_core::chat_recap::{recap, RecapFacts};
use ralph_server::chat::memory::Store;
use std::io::{self, Write};
use std::process::Command;
use std::time::Duration;

const GIT_LIMIT: Duration = Duration::from_secs(2);

pub(super) fn print_recap(
    store: &Store,
    workers: usize,
    style: SentStyle,
    width: usize,
    out: &mut dyn Write,
) -> io::Result<()> {
    let Ok((entries, _)) = store.load_log_quiet() else {
        return Ok(());
    };
    if entries.is_empty() {
        return Ok(());
    }
    let facts = RecapFacts {
        entries: &entries,
        now: chrono::Utc::now(),
        workers,
        unpushed: unpushed_commits().unwrap_or(0),
        width,
    };
    write_lines(&recap(&facts), style, out)
}

fn write_lines(lines: &[String], style: SentStyle, out: &mut dyn Write) -> io::Result<()> {
    lines
        .iter()
        .try_for_each(|line| writeln!(out, "{}", paint_recap_line(line, style)))
}

fn paint_recap_line(line: &str, style: SentStyle) -> String {
    line.strip_prefix("  you: ").map_or_else(
        || line.to_string(),
        |text| format!("  you: {}", style.paint(text)),
    )
}

fn unpushed_commits() -> Option<usize> {
    let mut command = Command::new("git");
    command.args(["--no-optional-locks", "rev-list", "--count", "@{u}..HEAD"]);
    stdout_within(&mut command, GIT_LIMIT)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::sent::SentStyle;

    #[test]
    fn write_lines_writes_each_painted_line() {
        let style = SentStyle::new(None);
        let lines = vec!["  you: /hold topic".to_string(), "  next: x".to_string()];
        let mut out = Vec::new();
        write_lines(&lines, style, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "  you: /hold topic\n  next: x\n"
        );
    }

    #[test]
    fn recap_styles_only_the_user_command() {
        let style = SentStyle::new(Some(reedline::Color::Magenta));
        assert_eq!(
            paint_recap_line("  you: /hold topic", style),
            format!(
                "  you: {} topic",
                reedline::Color::Magenta.bold().paint("/hold")
            )
        );
        for line in ["  next: /hold topic", "  you: text /hold", "  you: /tmp/x"] {
            assert_eq!(paint_recap_line(line, style), line);
        }
        assert_eq!(
            paint_recap_line("  you: /merge", SentStyle::new(None)),
            "  you: /merge"
        );
    }
}
