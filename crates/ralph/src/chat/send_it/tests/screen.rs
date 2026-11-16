use super::*;

fn screen() -> Screen {
    Screen {
        text: "plain".to_string(),
        styled: "\x1b[1mplain\x1b[0m".to_string(),
        shown: None,
    }
}

#[test]
fn a_screen_without_color_prints_the_plain_text() {
    assert_eq!(screen().for_terminal(false), "plain");
}

#[test]
fn a_screen_with_color_prints_the_styled_text() {
    assert_eq!(screen().for_terminal(true), "\x1b[1mplain\x1b[0m");
}
