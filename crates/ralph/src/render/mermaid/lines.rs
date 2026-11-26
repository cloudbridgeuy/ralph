pub(super) fn strip_inline_comment(line: &str) -> &str {
    let mut quoted = false;
    line.char_indices()
        .find(|&(at, c)| {
            quoted ^= c == '"';
            !quoted && line[at..].starts_with("%%")
        })
        .map_or(line, |(at, _)| &line[..at])
}

pub(super) fn body_lines(source: &str) -> impl Iterator<Item = &str> {
    source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"))
}
