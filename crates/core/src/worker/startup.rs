const TRUST_ANSWER: &str = "Yes, I trust this folder";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    Starting,
    TrustPrompt,
    Ready,
}

pub fn startup_state(pane: &str, ready: bool) -> Startup {
    if ready {
        Startup::Ready
    } else if pane.contains(TRUST_ANSWER) {
        Startup::TrustPrompt
    } else {
        Startup::Starting
    }
}

pub fn trust_key(pane: &str, previous: Option<&str>) -> Option<&'static str> {
    if previous != Some(pane) || !pane.contains(TRUST_ANSWER) {
        return None;
    }
    let selected = pane
        .lines()
        .find(|line| line.trim_start().starts_with('❯'))?;
    if selected.contains("No, exit") {
        Some("Down")
    } else if selected.contains(TRUST_ANSWER) {
        Some("Enter")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_key_answers_only_a_stable_trust_prompt() {
        let on_no = " Accessing workspace:\n /private/tmp/x\n ❯ No, exit\n   Yes, I trust this folder\n Enter to confirm · Esc to cancel\n";
        let on_yes = " Accessing workspace:\n /private/tmp/x\n   No, exit\n ❯ Yes, I trust this folder\n Enter to confirm · Esc to cancel\n";
        let ready = "❯ \n  ctrl+p commands\n";
        assert_eq!(trust_key(ready, Some(ready)), None);
        assert_eq!(trust_key(on_no, None), None);
        assert_eq!(trust_key(on_no, Some(on_yes)), None);
        assert_eq!(trust_key(on_no, Some(on_no)), Some("Down"));
        assert_eq!(trust_key(on_yes, Some(on_yes)), Some("Enter"));
    }

    #[test]
    fn startup_state_waits_for_the_ready_signal_and_spots_the_trust_prompt() {
        let trust = " Accessing workspace:\n ❯ No, exit\n   Yes, I trust this folder\n";
        assert_eq!(startup_state("", false), Startup::Starting);
        assert_eq!(startup_state(trust, false), Startup::TrustPrompt);
        assert_eq!(startup_state(trust, true), Startup::Ready);
        assert_eq!(
            startup_state(" ▐▛███▛█   Claude Code v2.1.295\n", false),
            Startup::Starting
        );
        assert_eq!(
            startup_state("  ctrl+p commands\n", false),
            Startup::Starting
        );
    }
}
