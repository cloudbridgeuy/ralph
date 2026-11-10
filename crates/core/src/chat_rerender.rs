use crate::chat_memory::{Entry, Role};

pub const RERENDER_USAGE: &str = "Usage: /rerender [N] (N is 1 or more; 1 is the last reply)";

pub fn parse_rerender_args(args: &str) -> Result<usize, String> {
    let mut words = args.split_whitespace();
    let Some(first) = words.next() else {
        return Ok(1);
    };
    match (first.parse::<usize>(), words.next()) {
        (Ok(n), None) if n >= 1 => Ok(n),
        _ => Err(RERENDER_USAGE.to_string()),
    }
}

pub fn nth_last_assistant(entries: &[Entry], n: usize) -> Result<&Entry, String> {
    let replies: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.role == Role::Assistant && !entry.is_corrupt())
        .collect();
    let count = replies.len();
    if count == 0 {
        return Err("/rerender: no assistant reply yet.".to_string());
    }
    count
        .checked_sub(n)
        .and_then(|index| replies.get(index).copied())
        .ok_or_else(|| format!("/rerender: no reply {n}; the chat has {count} assistant replies."))
}

pub fn rerender_header(n: usize) -> String {
    format!("reprint of reply {n}")
}

fn word_len(text: &str) -> usize {
    text.find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(text.len())
}

fn open_tag_len(rest: &str) -> Option<usize> {
    let after_prefix = rest.strip_prefix("<ralph-")?;
    let verb = word_len(after_prefix);
    let after_verb = after_prefix.get(verb..).filter(|_| verb > 0)?;
    let spaced = after_verb.trim_start();
    let value = spaced
        .strip_prefix("to=\"")
        .filter(|_| spaced.len() < after_verb.len())?;
    let tail = value.get(value.find('"')?..)?.strip_prefix("\">")?;
    Some(rest.len() - tail.len())
}

fn close_tag_len(rest: &str) -> Option<usize> {
    let after_prefix = rest.strip_prefix("</ralph-")?;
    let verb = word_len(after_prefix);
    let tail = after_prefix
        .get(verb..)
        .filter(|_| verb > 0)?
        .strip_prefix('>')?;
    Some(rest.len() - tail.len())
}

pub fn strip_directive_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        let tag = if ch == '<' {
            open_tag_len(rest).or_else(|| close_tag_len(rest))
        } else {
            None
        };
        let take = tag.unwrap_or(ch.len_utf8());
        if tag.is_none() {
            out.push(ch);
        }
        rest = rest.get(take..).unwrap_or_default();
    }
    out
}

#[cfg(test)]
mod tests;
