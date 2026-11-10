use super::*;
use crate::chat_rerender::RERENDER_USAGE;

fn route(line: &str) -> Route {
    route_input(line, 0, &EmptySubmit::default(), &BTreeSet::new())
}

#[test]
fn bare_rerender_routes_to_the_last_reply() {
    assert_eq!(route("/rerender"), Route::Rerender(Ok(1)));
}

#[test]
fn rerender_with_a_number_routes_to_that_reply() {
    assert_eq!(route("/rerender 3"), Route::Rerender(Ok(3)));
    assert_eq!(route("  /rerender   2  "), Route::Rerender(Ok(2)));
}

#[test]
fn bad_rerender_arguments_route_to_the_usage() {
    for line in ["/rerender 0", "/rerender x", "/rerender 1 2"] {
        assert_eq!(
            route(line),
            Route::Rerender(Err(RERENDER_USAGE.to_string())),
            "{line}"
        );
    }
}

#[test]
fn rerender_waits_for_the_turn_to_end() {
    assert!(!route("/rerender").runs_mid_turn());
}

#[test]
fn a_longer_command_name_is_not_rerender() {
    assert!(!matches!(route("/rerenderx"), Route::Rerender(_)));
}

#[test]
fn help_has_a_rerender_topic_with_its_row() {
    let text = crate::chat_help::help_topic(Some("rerender")).unwrap();
    assert!(text.starts_with("/rerender\n"), "{text}");
    assert!(text.contains("/rerender [N]"), "{text}");
}
