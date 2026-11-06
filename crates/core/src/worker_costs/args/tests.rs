use super::*;

fn parsed(text: &str) -> Option<CostsArgs> {
    parse_costs_args(&text.split_whitespace().collect::<Vec<_>>())
}

fn args(view: View, window: Window) -> Option<CostsArgs> {
    Some(CostsArgs { view, window })
}

#[test]
fn no_arguments_mean_the_harness_and_model_view_for_all_time() {
    assert_eq!(parsed(""), args(View::HarnessModel, Window::All));
    assert_eq!(CostsArgs::default(), parsed("").unwrap());
}

#[test]
fn each_view_name_selects_its_view() {
    assert_eq!(parsed("harness"), args(View::Harness, Window::All));
    assert_eq!(parsed("model"), args(View::HarnessModel, Window::All));
    assert_eq!(parsed("role"), args(View::Role, Window::All));
    assert_eq!(parsed("ticket"), args(View::Ticket, Window::All));
}

#[test]
fn each_window_name_selects_its_window_on_the_default_view() {
    assert_eq!(parsed("today"), args(View::HarnessModel, Window::Today));
    assert_eq!(parsed("7d"), args(View::HarnessModel, Window::Week));
    assert_eq!(parsed("all"), args(View::HarnessModel, Window::All));
}

#[test]
fn a_view_and_a_window_combine() {
    assert_eq!(parsed("role today"), args(View::Role, Window::Today));
    assert_eq!(parsed("ticket 7d"), args(View::Ticket, Window::Week));
    assert_eq!(parsed("model all"), args(View::HarnessModel, Window::All));
}

#[test]
fn unknown_extra_or_reordered_words_are_refused() {
    for text in [
        "monthly",
        "role monthly",
        "today role",
        "role role",
        "today 7d",
        "role today extra",
        "Role",
        "7D",
    ] {
        assert_eq!(parsed(text), None, "{text}");
    }
}
