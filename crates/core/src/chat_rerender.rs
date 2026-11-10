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

fn open_tag(rest: &str) -> Option<(usize, &str)> {
    let after_prefix = rest.strip_prefix("<ralph-")?;
    let verb = word_len(after_prefix);
    let after_verb = after_prefix.get(verb..).filter(|_| verb > 0)?;
    let spaced = after_verb.trim_start();
    let value = spaced
        .strip_prefix("to=\"")
        .filter(|_| spaced.len() < after_verb.len())?;
    let tail = value.get(value.find('"')?..)?.strip_prefix("\">")?;
    Some((rest.len() - tail.len(), after_prefix.get(..verb)?))
}

fn directive_len(rest: &str) -> Option<usize> {
    let (open, verb) = open_tag(rest)?;
    let close = format!("</ralph-{verb}>");
    let body = rest.get(open..)?;
    Some(open + body.find(&close)? + close.len())
}

pub fn strip_directive_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut removed = false;
    while let Some(ch) = rest.chars().next() {
        let span = if ch == '<' { directive_len(rest) } else { None };
        match span {
            Some(len) => {
                removed = true;
                rest = rest.get(len..).unwrap_or_default().trim_start_matches('\n');
            }
            None => {
                out.push(ch);
                rest = rest.get(ch.len_utf8()..).unwrap_or_default();
            }
        }
    }
    if removed {
        out.truncate(out.trim_end().len());
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reprint {
    pub header: String,
    pub markdown: String,
}

pub fn reprint(entries: &[Entry], n: usize) -> Result<Reprint, String> {
    nth_last_assistant(entries, n).map(|entry| Reprint {
        header: rerender_header(n),
        markdown: strip_directive_tags(&entry.text),
    })
}

#[cfg(test)]
mod tests;
