use super::*;

fn input(rendered: Option<usize>, new_cols: usize) -> ResizeInput {
    ResizeInput {
        rendered,
        new_cols,
        idle: true,
        main_view: true,
    }
}

#[test]
fn narrowing_at_an_idle_main_view_redraws() {
    assert!(should_redraw(&input(Some(255), 48)));
}

#[test]
fn widening_does_not_redraw() {
    assert!(!should_redraw(&input(Some(48), 255)));
}

#[test]
fn the_same_width_does_not_redraw() {
    assert!(!should_redraw(&input(Some(48), 48)));
}

#[test]
fn a_thread_view_never_redraws() {
    let thread = ResizeInput {
        main_view: false,
        ..input(Some(255), 48)
    };
    assert!(!should_redraw(&thread));
}

#[test]
fn a_running_turn_does_not_redraw() {
    let busy = ResizeInput {
        idle: false,
        ..input(Some(255), 48)
    };
    assert!(!should_redraw(&busy));
}

#[test]
fn no_rendered_reply_does_not_redraw() {
    assert!(!should_redraw(&input(None, 48)));
}

#[test]
fn a_reply_keeps_the_narrowest_chunk_width() {
    let after_wide_then_narrow = on_render(Some(on_render(None, 255)), 48);
    assert_eq!(after_wide_then_narrow, 48);
    assert!(!should_redraw(&input(Some(after_wide_then_narrow), 48)));
    assert_eq!(on_render(Some(on_render(None, 48)), 255), 48);
}

#[test]
fn a_phone_desktop_phone_round_trip_redraws_once() {
    let mut rendered = None;
    let mut redraws = 0;
    for step in [
        Step::Size(48),
        Step::Size(255),
        Step::Reply(255),
        Step::Size(48),
    ] {
        match step {
            Step::Reply(cols) => rendered = Some(on_render(None, cols)),
            Step::Size(cols) => {
                if should_redraw(&input(rendered, cols)) {
                    redraws += 1;
                    rendered = Some(cols);
                }
            }
        }
    }
    assert_eq!(redraws, 1);
    assert_eq!(rendered, Some(48));
}

enum Step {
    Size(usize),
    Reply(usize),
}

#[test]
fn a_burst_of_signals_acts_once() {
    let mut last = None;
    let mut acted = 0;
    for now in [0, 20, 40, 60, 90] {
        if debounce(last, now, DEBOUNCE_MS) {
            acted += 1;
            last = Some(now);
        }
    }
    assert_eq!(acted, 1);
}

#[test]
fn a_signal_after_the_window_acts_again() {
    assert!(debounce(Some(0), DEBOUNCE_MS, DEBOUNCE_MS));
    assert!(!debounce(Some(0), DEBOUNCE_MS - 1, DEBOUNCE_MS));
}
