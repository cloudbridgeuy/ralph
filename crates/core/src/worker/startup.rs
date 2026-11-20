const TRUST_ANSWERS: [&str; 2] = ["Yes, I trust this folder", "Yes, add it"];

fn shows_trust_answer(text: &str) -> bool {
    TRUST_ANSWERS.iter().any(|answer| text.contains(answer))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    Starting,
    TrustPrompt,
    Ready,
}

pub fn startup_state(pane: &str, ready: bool) -> Startup {
    if ready {
        Startup::Ready
    } else if shows_trust_answer(pane) {
        Startup::TrustPrompt
    } else {
        Startup::Starting
    }
}

pub fn trust_key(pane: &str, previous: Option<&str>) -> Option<&'static str> {
    if previous != Some(pane) || !shows_trust_answer(pane) {
        return None;
    }
    let selected = pane
        .lines()
        .find(|line| line.trim_start().starts_with('❯'))?;
    if selected.contains("No, exit") {
        Some("Down")
    } else if shows_trust_answer(selected) {
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

    const ADD_DIR_ON_NO: &str = "\n────────────────────────────────────────────────────────────────────────────────\n Adding a new directory:\n /private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpz76MtI/store/atta\n chments\n This session hasn't worked here before. Is this a directory you created or one\n you trust?\n Claude Code'll be able to read, edit, and execute files here. Its skills,\n commands, and agents load and can run commands, and so can the git settings of\n its repository. Your answer holds for this session.\n Security guide\n ❯ No, exit\n   Yes, add it\n Enter to confirm · Esc to cancel\n";
    const ADD_DIR_ON_YES: &str = "\n────────────────────────────────────────────────────────────────────────────────\n Adding a new directory:\n /private/var/folders/g9/4b8tv1yj0tnfsp21spqtx5zr0000gp/T/.tmpz76MtI/store/atta\n chments\n This session hasn't worked here before. Is this a directory you created or one\n you trust?\n Claude Code'll be able to read, edit, and execute files here. Its skills,\n commands, and agents load and can run commands, and so can the git settings of\n its repository. Your answer holds for this session.\n Security guide\n   No, exit\n ❯ Yes, add it\n Enter to confirm · Esc to cancel\n";

    #[test]
    fn add_dir_dialog_is_a_trust_prompt() {
        assert_eq!(startup_state(ADD_DIR_ON_NO, false), Startup::TrustPrompt);
        assert_eq!(startup_state(ADD_DIR_ON_YES, false), Startup::TrustPrompt);
        assert_eq!(startup_state(ADD_DIR_ON_NO, true), Startup::Ready);
    }

    #[test]
    fn add_dir_dialog_is_answered_only_when_stable() {
        assert_eq!(trust_key(ADD_DIR_ON_NO, Some(ADD_DIR_ON_NO)), Some("Down"));
        assert_eq!(
            trust_key(ADD_DIR_ON_YES, Some(ADD_DIR_ON_YES)),
            Some("Enter")
        );
        assert_eq!(trust_key(ADD_DIR_ON_NO, None), None);
        assert_eq!(trust_key(ADD_DIR_ON_NO, Some(ADD_DIR_ON_YES)), None);
    }
}
