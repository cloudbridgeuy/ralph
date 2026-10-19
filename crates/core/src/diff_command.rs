//! `/diff` target, argv and shell-word splitting. No I/O.

use crate::changes::{ScopeQuery, TurnWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffPager {
    Git,
    Delta,
    Hunk,
}

/// What `/diff` shows: the working tree (`now`) or a snapshot range, plus
/// the typed git diff args.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    Now(Vec<String>),
    Range {
        scope: ScopeQuery,
        extra: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffArgs {
    Target(DiffTarget),
    Usage(&'static str),
    UnclosedQuote,
}

pub const UNCLOSED_DIFF_QUOTE: &str = "/diff: unclosed quote";
pub const WINDOW_USAGE: &str = "/diff: last and ago need a number of 1 or more";
pub const WORKER_USAGE: &str = "/diff: worker needs a worker id";

/// `git diff` argv. `revs` (snapshot range) come right after `diff`, then
/// `diff_args` (from config), then `extra` (typed).
/// `delta` and `hunk` set `core.pager` for this call only; `git` sets nothing.
pub fn diff_command(
    pager: DiffPager,
    revs: &[String],
    diff_args: &[String],
    extra: &[String],
) -> Vec<String> {
    let mut command = Vec::with_capacity(4 + revs.len() + diff_args.len() + extra.len());
    command.push("git".to_string());
    if let Some(value) = pager_config(pager) {
        command.push("-c".to_string());
        command.push(value.to_string());
    }
    command.push("diff".to_string());
    command.extend(revs.iter().cloned());
    command.extend(diff_args.iter().cloned());
    command.extend(extra.iter().cloned());
    command
}

pub fn diff_quiet_command(revs: &[String], diff_args: &[String], extra: &[String]) -> Vec<String> {
    let mut command = diff_command(DiffPager::Git, revs, diff_args, extra);
    command.insert(2, "--quiet".to_string());
    command
}

pub fn range_is_empty(exit_code: Option<i32>) -> bool {
    exit_code == Some(0)
}

fn pager_config(pager: DiffPager) -> Option<&'static str> {
    match pager {
        DiffPager::Git => None,
        DiffPager::Delta => Some("core.pager=delta"),
        DiffPager::Hunk => Some("core.pager=hunk pager"),
    }
}

/// Parse a `/diff` tail: split it into words (quotes and backslash escapes
/// join; there is no `$` expansion or globbing; a trailing backslash is kept),
/// then pick the target. `now`, `last N`, `ago N` and `worker <id> [last N|ago N]`
/// lead the words; the rest are git diff args.
pub fn parse_diff_args(input: &str) -> DiffArgs {
    match split_shell_words(input) {
        Ok(words) => match parse_target(&words) {
            Ok(target) => DiffArgs::Target(target),
            Err(usage) => DiffArgs::Usage(usage),
        },
        Err(UnclosedQuote) => DiffArgs::UnclosedQuote,
    }
}

fn parse_target(words: &[String]) -> Result<DiffTarget, &'static str> {
    match words.first().map(String::as_str) {
        Some("now") => Ok(DiffTarget::Now(words[1..].to_vec())),
        Some("worker") => {
            let worker = words
                .get(1)
                .filter(|id| !id.starts_with('-') && !matches!(id.as_str(), "last" | "ago" | "now"))
                .ok_or(WORKER_USAGE)?;
            let (window, extra) = take_window(&words[2..])?;
            Ok(DiffTarget::Range {
                scope: ScopeQuery::Worker {
                    worker: worker.clone(),
                    window,
                },
                extra,
            })
        }
        _ => {
            let (window, extra) = take_window(words)?;
            Ok(DiffTarget::Range {
                scope: ScopeQuery::Chat(window.unwrap_or(TurnWindow::Last(1))),
                extra,
            })
        }
    }
}

fn take_window(words: &[String]) -> Result<(Option<TurnWindow>, Vec<String>), &'static str> {
    let constructor: fn(usize) -> TurnWindow = match words.first().map(String::as_str) {
        Some("last") => TurnWindow::Last,
        Some("ago") => TurnWindow::Ago,
        _ => return Ok((None, words.to_vec())),
    };
    let count = words
        .get(1)
        .and_then(|word| word.parse::<usize>().ok())
        .filter(|count| *count >= 1)
        .ok_or(WINDOW_USAGE)?;
    Ok((Some(constructor(count)), words[2..].to_vec()))
}

struct UnclosedQuote;

fn split_shell_words(input: &str) -> Result<Vec<String>, UnclosedQuote> {
    let chars: Vec<char> = input.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut word = String::new();
        while i < chars.len() && !chars[i].is_whitespace() {
            match chars[i] {
                '\\' => {
                    if i + 1 >= chars.len() {
                        word.push('\\');
                        i += 1;
                    } else {
                        word.push(chars[i + 1]);
                        i += 2;
                    }
                }
                '\'' => {
                    i = push_single(&chars, i + 1, &mut word)?;
                }
                '"' => {
                    i = push_double(&chars, i + 1, &mut word)?;
                }
                c => {
                    word.push(c);
                    i += 1;
                }
            }
        }
        words.push(word);
    }
    Ok(words)
}

fn push_single(chars: &[char], mut i: usize, word: &mut String) -> Result<usize, UnclosedQuote> {
    let start = i;
    while i < chars.len() && chars[i] != '\'' {
        i += 1;
    }
    if i >= chars.len() {
        return Err(UnclosedQuote);
    }
    word.extend(chars[start..i].iter());
    Ok(i + 1)
}

fn push_double(chars: &[char], mut i: usize, word: &mut String) -> Result<usize, UnclosedQuote> {
    while i < chars.len() && chars[i] != '"' {
        if chars[i] == '\\' {
            if i + 1 >= chars.len() {
                return Err(UnclosedQuote);
            }
            word.push(chars[i + 1]);
            i += 2;
        } else {
            word.push(chars[i]);
            i += 1;
        }
    }
    if i >= chars.len() {
        return Err(UnclosedQuote);
    }
    Ok(i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn diff_command_maps_pagers_and_keeps_arg_order() {
        let revs = words(&["r1", "r2"]);
        let config = words(&["--stat", "--"]);
        let extra = words(&["-U1", "a b"]);
        assert_eq!(
            diff_command(DiffPager::Git, &revs, &config, &extra),
            words(&["git", "diff", "r1", "r2", "--stat", "--", "-U1", "a b"])
        );
        assert_eq!(
            diff_command(DiffPager::Delta, &revs, &config, &extra),
            words(&[
                "git",
                "-c",
                "core.pager=delta",
                "diff",
                "r1",
                "r2",
                "--stat",
                "--",
                "-U1",
                "a b"
            ])
        );
        assert_eq!(
            diff_command(DiffPager::Hunk, &[], &[], &[]),
            words(&["git", "-c", "core.pager=hunk pager", "diff"])
        );
        assert_eq!(
            diff_command(DiffPager::Hunk, &revs[..1], &[], &extra[..1]),
            words(&["git", "-c", "core.pager=hunk pager", "diff", "r1", "-U1"])
        );
    }

    #[test]
    fn diff_quiet_command_puts_quiet_right_after_diff() {
        assert_eq!(
            diff_quiet_command(
                &words(&["r1", "r2"]),
                &words(&["--stat", "--"]),
                &words(&["a"])
            ),
            words(&["git", "diff", "--quiet", "r1", "r2", "--stat", "--", "a"])
        );
        assert_eq!(
            diff_quiet_command(&[], &[], &[]),
            words(&["git", "diff", "--quiet"])
        );
    }

    #[test]
    fn range_is_empty_only_for_exit_zero() {
        assert!(range_is_empty(Some(0)));
        assert!(!range_is_empty(Some(1)));
        assert!(!range_is_empty(Some(129)));
        assert!(!range_is_empty(None));
    }

    fn chat_last(count: usize, extra: &[&str]) -> DiffArgs {
        range(ScopeQuery::Chat(TurnWindow::Last(count)), extra)
    }

    fn range(scope: ScopeQuery, extra: &[&str]) -> DiffArgs {
        DiffArgs::Target(DiffTarget::Range {
            scope,
            extra: words(extra),
        })
    }

    fn worker(id: &str, window: Option<TurnWindow>, extra: &[&str]) -> DiffArgs {
        range(
            ScopeQuery::Worker {
                worker: id.to_string(),
                window,
            },
            extra,
        )
    }

    #[test]
    fn parse_diff_args_picks_the_range() {
        assert_eq!(parse_diff_args(""), chat_last(1, &[]));
        assert_eq!(parse_diff_args("last 3"), chat_last(3, &[]));
        assert_eq!(
            parse_diff_args("ago 2 --stat"),
            range(ScopeQuery::Chat(TurnWindow::Ago(2)), &["--stat"])
        );
        assert_eq!(
            parse_diff_args("last 2 -U1 -- a"),
            chat_last(2, &["-U1", "--", "a"])
        );
        assert_eq!(
            parse_diff_args("-U1 -- a"),
            chat_last(1, &["-U1", "--", "a"])
        );
        assert_eq!(parse_diff_args("turn 3"), chat_last(1, &["turn", "3"]));
        assert_eq!(parse_diff_args("worker w1"), worker("w1", None, &[]));
        assert_eq!(
            parse_diff_args("worker w1 --stat"),
            worker("w1", None, &["--stat"])
        );
        assert_eq!(
            parse_diff_args("worker w1 last 2 --stat"),
            worker("w1", Some(TurnWindow::Last(2)), &["--stat"])
        );
        assert_eq!(
            parse_diff_args("worker w1 ago 1"),
            worker("w1", Some(TurnWindow::Ago(1)), &[])
        );
    }

    #[test]
    fn parse_diff_args_now_keeps_the_old_working_tree_words() {
        assert_eq!(
            parse_diff_args("now"),
            DiffArgs::Target(DiffTarget::Now(vec![]))
        );
        assert_eq!(
            parse_diff_args("now -U1 -- \"a b\""),
            DiffArgs::Target(DiffTarget::Now(words(&["-U1", "--", "a b"])))
        );
    }

    #[test]
    fn parse_diff_args_rejects_bad_counts_and_missing_worker() {
        for input in [
            "last",
            "last 0",
            "ago 0",
            "last x",
            "ago -1",
            "worker w last",
            "worker w ago 0",
        ] {
            assert_eq!(
                parse_diff_args(input),
                DiffArgs::Usage(WINDOW_USAGE),
                "{input}"
            );
        }
        for input in [
            "worker",
            "worker --stat",
            "worker last 2",
            "worker ago 1",
            "worker now",
            "worker last",
        ] {
            assert_eq!(
                parse_diff_args(input),
                DiffArgs::Usage(WORKER_USAGE),
                "{input}"
            );
        }
        assert_eq!(parse_diff_args("last 'a"), DiffArgs::UnclosedQuote);
    }

    #[test]
    fn parse_diff_args_keeps_quotes_and_rejects_unclosed() {
        let cases = [
            ("", &[][..]),
            ("-U1 -- a", &["-U1", "--", "a"][..]),
            ("-- \"a b\"", &["--", "a b"][..]),
            ("-- 'a b'", &["--", "a b"][..]),
            ("-- a\\ b", &["--", "a b"][..]),
            ("-- \"a\\\"b\"", &["--", "a\"b"][..]),
            ("foo'bar'", &["foobar"][..]),
            ("''", &[""][..]),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_diff_args(input), chat_last(1, expected));
        }
        assert_eq!(parse_diff_args("'a"), DiffArgs::UnclosedQuote);
        assert_eq!(parse_diff_args("\"a"), DiffArgs::UnclosedQuote);
    }
}
