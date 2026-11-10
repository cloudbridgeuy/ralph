use super::*;

#[test]
fn bare_rerender_means_the_last_reply() {
    assert_eq!(parse_rerender_args(""), Ok(1));
    assert_eq!(parse_rerender_args("   "), Ok(1));
}

#[test]
fn a_positive_number_selects_that_reply() {
    assert_eq!(parse_rerender_args("1"), Ok(1));
    assert_eq!(parse_rerender_args(" 3 "), Ok(3));
}

#[test]
fn zero_words_and_extra_arguments_give_the_usage() {
    let usage = Err(RERENDER_USAGE.to_string());
    for args in ["0", "x", "-1", "1.5", "2 3", "1 x", "last"] {
        assert_eq!(parse_rerender_args(args), usage, "{args}");
    }
    assert_eq!(
        RERENDER_USAGE,
        "Usage: /rerender [N] (N is 1 or more; 1 is the last reply)"
    );
}
