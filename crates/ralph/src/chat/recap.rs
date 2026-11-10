use crate::chat::bounded::stdout_within;
use crate::chat::prompt::screen_columns;
use crate::chat::sent::SentStyle;
use ralph_core::chat_recap::{recap, RecapFacts};
use ralph_server::chat::memory::Store;
use std::process::Command;
use std::time::Duration;

const GIT_LIMIT: Duration = Duration::from_secs(2);

pub(super) fn print_recap(store: &Store, workers: usize, style: SentStyle) {
    let Ok((entries, _)) = store.load_log_quiet() else {
        return;
    };
    if entries.is_empty() {
        return;
    }
    let facts = RecapFacts {
        entries: &entries,
        now: chrono::Utc::now(),
        workers,
        unpushed: unpushed_commits().unwrap_or(0),
        width: screen_columns(),
    };
    recap(&facts)
        .iter()
        .for_each(|line| println!("{}", paint_recap_line(line, style)));
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
    fn recap_styles_only_the_user_command() {
        let style = SentStyle::new(Some(reedline::Color::Magenta));
        assert_eq!(
            paint_recap_line("  you: /fork topic", style),
            format!(
                "  you: {} topic",
                reedline::Color::Magenta.bold().paint("/fork")
            )
        );
        for line in ["  next: /fork topic", "  you: text /fork", "  you: /tmp/x"] {
            assert_eq!(paint_recap_line(line, style), line);
        }
        assert_eq!(
            paint_recap_line("  you: /merge", SentStyle::new(None)),
            "  you: /merge"
        );
    }
}
