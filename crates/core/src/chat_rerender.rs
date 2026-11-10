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

#[cfg(test)]
mod tests;
