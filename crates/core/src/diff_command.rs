//! `/diff` argv and shell-word splitting. No I/O.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffPager {
    Git,
    Delta,
    Hunk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffArgs {
    Words(Vec<String>),
    UnclosedQuote,
}

pub const UNCLOSED_DIFF_QUOTE: &str = "/diff: unclosed quote";

/// `git diff` argv. `diff_args` (from config) come before `extra` (typed).
/// `delta` and `hunk` set `core.pager` for this call only; `git` sets nothing.
pub fn diff_command(pager: DiffPager, diff_args: &[String], extra: &[String]) -> Vec<String> {
    let mut command = Vec::with_capacity(4 + diff_args.len() + extra.len());
    command.push("git".to_string());
    if let Some(value) = pager_config(pager) {
        command.push("-c".to_string());
        command.push(value.to_string());
    }
    command.push("diff".to_string());
    command.extend(diff_args.iter().cloned());
    command.extend(extra.iter().cloned());
    command
}

fn pager_config(pager: DiffPager) -> Option<&'static str> {
    match pager {
        DiffPager::Git => None,
        DiffPager::Delta => Some("core.pager=delta"),
        DiffPager::Hunk => Some("core.pager=hunk pager"),
    }
}

/// Split a `/diff` tail into words. Quotes and backslash escapes join; there
/// is no `$` expansion or globbing. A trailing backslash is kept.
pub fn parse_diff_args(input: &str) -> DiffArgs {
    match split_shell_words(input) {
        Ok(words) => DiffArgs::Words(words),
        Err(UnclosedQuote) => DiffArgs::UnclosedQuote,
    }
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
        let config = words(&["--stat"]);
        let extra = words(&["-U1", "--", "a b"]);
        assert_eq!(
            diff_command(DiffPager::Git, &config, &extra),
            words(&["git", "diff", "--stat", "-U1", "--", "a b"])
        );
        assert_eq!(
            diff_command(DiffPager::Delta, &config, &extra),
            words(&[
                "git",
                "-c",
                "core.pager=delta",
                "diff",
                "--stat",
                "-U1",
                "--",
                "a b"
            ])
        );
        assert_eq!(
            diff_command(DiffPager::Hunk, &[], &[]),
            words(&["git", "-c", "core.pager=hunk pager", "diff"])
        );
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
            assert_eq!(parse_diff_args(input), DiffArgs::Words(words(expected)));
        }
        assert_eq!(parse_diff_args("'a"), DiffArgs::UnclosedQuote);
        assert_eq!(parse_diff_args("\"a"), DiffArgs::UnclosedQuote);
    }
}
