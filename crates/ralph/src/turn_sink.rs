use crate::spinner::SpinnerContext;
use crate::subprocess::Stream;
use crate::ui_theme::UiColors;
use crate::warn::warning_line;
use ralph_core::turn_event::{TurnEvent, Wait};
use termimad::MadSkin;

pub(crate) fn rule(skin: &MadSkin, width: usize) -> String {
    skin.text("---", Some(width)).to_string()
}

pub(crate) fn rule_skin(ui: &UiColors) -> MadSkin {
    let mut skin = MadSkin::default();
    skin.horizontal_rule.set_fg(ui.rule);
    skin
}

pub(crate) fn written(event: &TurnEvent, ui: &UiColors, width: usize) -> Option<(Stream, String)> {
    match event {
        TurnEvent::Rule => Some((Stream::Stdout, rule(&rule_skin(ui), width))),
        TurnEvent::Warning(message) => Some((Stream::Stderr, warning_line(message))),
        TurnEvent::Output(text) => Some((Stream::Stdout, text.clone())),
        TurnEvent::Stderr(text) => Some((Stream::Stderr, text.clone())),
        TurnEvent::Busy(_) | TurnEvent::Resume(_) | TurnEvent::Idle => None,
    }
}

pub(crate) fn context(wait: Wait) -> SpinnerContext {
    match wait {
        Wait::Response => SpinnerContext::WaitingForResponse,
        Wait::Memory => SpinnerContext::WaitingForMemory,
        Wait::Thinking => SpinnerContext::Thinking,
        Wait::Tool => SpinnerContext::WaitingForTool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> UiColors {
        UiColors::default()
    }

    #[test]
    fn a_rule_is_the_themed_rule_at_the_width_on_stdout() {
        let expected = rule(&rule_skin(&ui()), 20);
        assert_eq!(
            written(&TurnEvent::Rule, &ui(), 20),
            Some((Stream::Stdout, expected))
        );
    }

    #[test]
    fn a_warning_is_a_warning_line_on_stderr() {
        assert_eq!(
            written(&TurnEvent::Warning("Turn cancelled.".into()), &ui(), 20),
            Some((Stream::Stderr, warning_line("Turn cancelled.")))
        );
    }

    #[test]
    fn output_and_stderr_pass_through_on_their_streams() {
        assert_eq!(
            written(&TurnEvent::Output("hi".into()), &ui(), 20),
            Some((Stream::Stdout, "hi".to_string()))
        );
        assert_eq!(
            written(&TurnEvent::Stderr("oops\n".into()), &ui(), 20),
            Some((Stream::Stderr, "oops\n".to_string()))
        );
    }

    #[test]
    fn spinner_events_write_nothing() {
        for event in [
            TurnEvent::Busy(Wait::Response),
            TurnEvent::Resume(Wait::Thinking),
            TurnEvent::Idle,
        ] {
            assert_eq!(written(&event, &ui(), 20), None);
        }
    }

    #[test]
    fn each_wait_shows_its_spinner_message() {
        assert_eq!(context(Wait::Response), SpinnerContext::WaitingForResponse);
        assert_eq!(context(Wait::Memory), SpinnerContext::WaitingForMemory);
        assert_eq!(context(Wait::Thinking), SpinnerContext::Thinking);
        assert_eq!(context(Wait::Tool), SpinnerContext::WaitingForTool);
    }
}
