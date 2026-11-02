use crate::chat::bounded::stdout_within;
use crate::chat::memory::Store;
use crate::chat::prompt::screen_columns;
use ralph_core::chat_recap::{recap, RecapFacts};
use std::process::Command;
use std::time::Duration;

const GIT_LIMIT: Duration = Duration::from_secs(2);

pub(super) fn print_recap(store: &Store, workers: usize) {
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
    recap(&facts).iter().for_each(|line| println!("{line}"));
}

fn unpushed_commits() -> Option<usize> {
    let mut command = Command::new("git");
    command.args(["--no-optional-locks", "rev-list", "--count", "@{u}..HEAD"]);
    stdout_within(&mut command, GIT_LIMIT)?.parse().ok()
}
