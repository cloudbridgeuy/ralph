use super::*;
use crate::chat_help::Command;
use crate::chat_thread::ThreadName;
use crate::chat_thread_cmd::{parse_thread_args, ThreadLine};

fn texts(found: Option<SpecCompletion>) -> Vec<String> {
    found.map_or_else(Vec::new, |found| {
        found.values.into_iter().map(|value| value.text).collect()
    })
}

#[test]
fn a_slash_offers_every_command_name_with_a_space() {
    let found = spec_completions("/", 1, &CompletionFacts::default());
    assert_eq!(texts(found.clone()).len(), Command::ALL.len());
    assert!(found.is_some_and(|found| found.values.iter().all(|value| value.append_space)));
}

#[test]
fn a_partial_command_name_is_replaced_from_its_slash() {
    let found = spec_completions("  /th", 5, &CompletionFacts::default());
    assert_eq!(found.as_ref().map(|found| found.span.clone()), Some(2..5));
    assert_eq!(texts(found), ["/thread"]);
}

#[test]
fn no_space_is_added_before_existing_whitespace() {
    let found = spec_completions("/th x", 3, &CompletionFacts::default());
    assert_eq!(
        found.map(|found| found.values),
        Some(vec![SpecValue {
            text: "/thread".to_string(),
            append_space: false,
        }])
    );
}

#[test]
fn paths_and_prose_are_left_to_other_completers() {
    for line in ["/Us", "/Users/", "/thread/x", "see /th", "th", ""] {
        let found = spec_completions(line, line.len(), &CompletionFacts::default());
        assert_eq!(found, None, "{line}");
    }
}

fn complete(line: &str, current: Option<&str>) -> Option<Vec<String>> {
    let current = current.map(|name| ThreadName::parse(name).unwrap());
    let threads = ["auth", "side", "sub"].map(String::from);
    let workers = ["w1", "w2"].map(String::from);
    let facts = CompletionFacts {
        current: current.as_ref(),
        threads: &threads,
        workers: &workers,
    };
    let found = spec_completions(line, line.len(), &facts);
    found.map(|found| texts(Some(found)))
}

fn words(list: &[&str]) -> Option<Vec<String>> {
    Some(list.iter().map(ToString::to_string).collect())
}

#[test]
fn switch_completion_lists_open_threads_without_the_current_one() {
    assert_eq!(
        complete("/thread switch ", Some("side")),
        words(&["auth", "sub"])
    );
    assert_eq!(complete("/thread switch s", None), words(&["side", "sub"]));
}

#[test]
fn subcommand_completion_offers_main_only_inside_a_thread() {
    let inside = [
        "drop", "ls", "main", "merge", "open", "park", "pull", "revive", "switch",
    ];
    assert_eq!(complete("/thread ", Some("side")), words(&inside));
    assert_eq!(
        complete("/thread m", Some("side")),
        words(&["main", "merge"])
    );
    assert_eq!(complete("/thread m", None), words(&["merge"]));
}

#[test]
fn completion_stops_after_the_name_and_where_nothing_is_offered() {
    for line in [
        "/thread switch auth ",
        "/thread park ",
        "/thread open ",
        "/thread revive ",
        "/main ",
    ] {
        assert_eq!(complete(line, Some("side")), None, "{line}");
    }
}

#[test]
fn subcommands_take_a_space_and_thread_names_do_not() {
    let spaced = |line: &str| {
        let threads = ["auth".to_string()];
        let facts = CompletionFacts {
            threads: &threads,
            ..CompletionFacts::default()
        };
        spec_completions(line, line.len(), &facts)
            .map(|found| (found.span, found.values[0].append_space))
    };
    assert_eq!(spaced("/thread sw"), Some((8..10, true)));
    assert_eq!(spaced("/thread switch au"), Some((15..17, false)));
}

#[test]
fn open_offers_flags_once_a_name_or_a_dash_is_typed() {
    let all = ["--auto-drop", "--auto-merge", "--model"];
    for line in ["/thread open x ", "/thread open x --", "/thread open --"] {
        assert_eq!(complete(line, None), words(&all), "{line}");
    }
}

#[test]
fn used_and_exclusive_open_flags_are_not_offered() {
    assert_eq!(
        complete("/thread open x --auto-merge --", None),
        words(&["--model"])
    );
    assert_eq!(
        complete("/thread open x --model opus --", None),
        words(&["--auto-drop", "--auto-merge"])
    );
    for line in ["/thread open x y --", "/thread open x --nope --"] {
        assert_eq!(complete(line, None), None, "{line}");
    }
}

#[test]
fn model_values_are_the_aliases() {
    assert_eq!(
        complete("/thread open opus --model ", None),
        words(&["haiku", "opus", "sonnet"])
    );
}

#[test]
fn merge_pull_park_and_drop_name_open_threads_only_in_main() {
    let all = ["auth", "side", "sub"];
    for sub in ["pull", "park", "drop", "merge"] {
        let line = format!("/thread {sub} ");
        assert_eq!(complete(&line, None), words(&all), "{line}");
        assert_eq!(complete(&line, Some("side")), None, "{line}");
    }
    assert_eq!(complete("/thread drop auth ", None), None);
}

#[test]
fn merge_offers_view_once() {
    for line in ["/thread merge auth ", "/thread merge --v"] {
        assert_eq!(complete(line, None), words(&["--view"]), "{line}");
    }
    for line in [
        "/thread merge auth --view ",
        "/thread merge --rebase auth --",
    ] {
        assert_eq!(complete(line, None), None, "{line}");
    }
}

#[test]
fn every_offered_flag_is_accepted_by_the_parser() {
    for line in [
        "/thread open x ",
        "/thread open x --auto-drop ",
        "/thread open x --model opus ",
        "/thread merge x ",
    ] {
        let offered = complete(line, None).unwrap_or_default();
        assert!(!offered.is_empty(), "{line}");
        for flag in offered {
            let value = if flag == "--model" { " opus" } else { "" };
            let typed = format!("{line}{flag}{value}");
            let args = typed.strip_prefix("/thread").unwrap();
            assert!(
                matches!(parse_thread_args(args), ThreadLine::Op(Ok(_))),
                "{typed}"
            );
        }
    }
}

#[test]
fn command_words_are_offered_in_the_first_position() {
    let cases: [(&str, &[&str]); 7] = [
        ("/inbox ", &["drop", "list", "next", "peek"]),
        ("/watch ", &["list", "start", "stop"]),
        ("/watch start ", &["--persistent"]),
        ("/schedule ", &["cancel", "list"]),
        ("/stage ", &["add", "done", "edit", "raw"]),
        ("/stage add ", &["decision", "followup", "owed"]),
        (
            "/send-it ",
            &["edit", "go", "off", "save", "status", "templates"],
        ),
    ];
    for (line, offered) in cases {
        assert_eq!(complete(line, None), words(offered), "{line}");
    }
}

#[test]
fn ids_and_free_text_are_not_completed() {
    for line in [
        "/inbox peek ",
        "/watch stop ",
        "/watch start make ",
        "/schedule cancel ",
        "/stage done ",
        "/stage raw ",
        "/send-it fix the ",
    ] {
        assert_eq!(complete(line, None), None, "{line}");
    }
}

#[test]
fn diff_offers_its_words_then_worker_ids_then_windows() {
    assert_eq!(
        complete("/diff ", None),
        words(&["ago", "last", "now", "worker"])
    );
    assert_eq!(complete("/diff worker ", None), words(&["w1", "w2"]));
    assert_eq!(complete("/diff worker w1 ", None), words(&["ago", "last"]));
    for line in ["/diff now ", "/diff worker last ", "/diff ./sr"] {
        assert_eq!(complete(line, None), None, "{line}");
    }
}

#[test]
fn workers_offers_ids_and_the_costs_words() {
    let views_and_windows = ["7d", "all", "harness", "model", "role", "ticket", "today"];
    assert_eq!(complete("/workers ", None), words(&["costs", "w1", "w2"]));
    assert_eq!(complete("/workers costs ", None), words(&views_and_windows));
    assert_eq!(
        complete("/workers costs role ", None),
        words(&["7d", "all", "today"])
    );
    assert_eq!(complete("/workers costs today ", None), None);
    assert_eq!(complete("/workers w1 ", None), None);
}

#[test]
fn answer_completes_only_the_worker() {
    assert_eq!(complete("/answer w", None), words(&["w1", "w2"]));
    assert_eq!(complete("/answer w1 ", None), None);
}

#[test]
fn read_offers_modes_and_the_cursor_flag_after_an_id() {
    assert_eq!(
        complete("/read 4-9 ", None),
        words(&["--cursor", "raw", "summary"])
    );
    assert_eq!(complete("/read 4-9 raw ", None), words(&["--cursor"]));
    for line in ["/read ", "/read --", "/read 4 --cursor ", "/read 4 raw x "] {
        assert_eq!(complete(line, None), None, "{line}");
    }
}

#[test]
fn search_and_rules_offer_the_cursor_flag_where_it_parses() {
    for line in ["/search retry ", "/search a b --", "/rules ", "/rules --c"] {
        assert_eq!(complete(line, None), words(&["--cursor"]), "{line}");
    }
    for line in [
        "/search ",
        "/search a --cursor c ",
        "/rules x ",
        "/recall a ",
    ] {
        assert_eq!(complete(line, None), None, "{line}");
    }
}
