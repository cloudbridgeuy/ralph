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

#[cfg(test)]
mod tests;
