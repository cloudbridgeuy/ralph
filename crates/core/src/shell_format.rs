const INDENT: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Sep {
    None,
    Semi,
    And,
    Or,
    Background,
    Pipe,
}

impl Sep {
    fn suffix(self, fence: bool) -> &'static str {
        match self {
            Sep::None => "",
            Sep::Semi if fence => ";",
            Sep::Semi => "",
            Sep::And => " &&",
            Sep::Or => " ||",
            Sep::Background => " &",
            Sep::Pipe => " |",
        }
    }

    fn is_chain(self) -> bool {
        matches!(self, Sep::And | Sep::Or)
    }
}

#[derive(Clone)]
struct Stage {
    text: String,
    flags: Vec<usize>,
}

fn join_stages(stages: &[Stage]) -> String {
    stages
        .iter()
        .map(|stage| stage.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
}

struct Segment {
    stages: Vec<Stage>,
    sep: Sep,
}

#[derive(PartialEq)]
enum Ctx {
    Double,
    Paren,
    DollarBrace,
    Backtick,
}

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Loop,
    If,
}

struct Scanner {
    segments: Vec<Segment>,
    stages: Vec<Stage>,
    current: String,
    flags: Vec<usize>,
    options_ended: bool,
}

impl Scanner {
    fn take_stage(&mut self) -> Option<()> {
        let leading = self.current.len() - self.current.trim_start().len();
        let stage = Stage {
            text: self.current.trim().to_string(),
            flags: std::mem::take(&mut self.flags)
                .into_iter()
                .map(|offset| offset - leading)
                .collect(),
        };
        self.current.clear();
        self.options_ended = false;
        if stage.text.is_empty() {
            return None;
        }
        self.stages.push(stage);
        Some(())
    }

    fn end_segment(&mut self, sep: Sep) -> Option<()> {
        self.take_stage()?;
        self.segments.push(Segment {
            stages: std::mem::take(&mut self.stages),
            sep,
        });
        Some(())
    }

    fn finish(mut self) -> Option<Vec<Segment>> {
        if self.current.trim().is_empty() && self.stages.is_empty() {
            let last = self.segments.last()?;
            return match last.sep {
                Sep::None | Sep::Semi | Sep::Background | Sep::Pipe => Some(self.segments),
                Sep::And | Sep::Or => None,
            };
        }
        self.end_segment(Sep::None)?;
        Some(self.segments)
    }
}

fn is_word_start(prev: Option<char>) -> bool {
    prev.is_none_or(|p| p.is_whitespace() || matches!(p, ';' | '&' | '|' | '('))
}

fn scan(chars: &[char]) -> Option<Vec<Segment>> {
    let mut scanner = Scanner {
        segments: Vec::new(),
        stages: Vec::new(),
        current: String::new(),
        flags: Vec::new(),
        options_ended: false,
    };
    let mut stack: Vec<Ctx> = Vec::new();
    let mut groups = 0usize;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let prev = i.checked_sub(1).map(|p| chars[p]);
        if c == '\\' {
            let escaped = next?;
            scanner.current.push(c);
            scanner.current.push(escaped);
            i += 2;
            continue;
        }
        let in_double = stack.last() == Some(&Ctx::Double);
        if c == '\'' && !in_double {
            let ansi = prev == Some('$')
                && chars[..i - 1]
                    .iter()
                    .rev()
                    .take_while(|&&c| c == '\\')
                    .count()
                    % 2
                    == 0;
            let mut close = i + 1;
            loop {
                let quoted = *chars.get(close)?;
                if quoted == '\'' {
                    break;
                }
                close += if ansi && quoted == '\\' { 2 } else { 1 };
            }
            scanner.current.extend(&chars[i..=close]);
            i = close + 1;
            continue;
        }
        if in_double {
            match (c, next) {
                ('"', _) => {
                    stack.pop();
                }
                ('$', Some('(')) => {
                    stack.push(Ctx::Paren);
                    scanner.current.push_str("$(");
                    i += 2;
                    continue;
                }
                ('$', Some('{')) => {
                    stack.push(Ctx::DollarBrace);
                    scanner.current.push_str("${");
                    i += 2;
                    continue;
                }
                ('`', _) => stack.push(Ctx::Backtick),
                _ => {}
            }
            scanner.current.push(c);
            i += 1;
            continue;
        }
        let mut consumed = !stack.is_empty();
        match c {
            '"' => {
                stack.push(Ctx::Double);
                consumed = true;
            }
            '(' => {
                stack.push(Ctx::Paren);
                consumed = true;
            }
            ')' => {
                if stack.pop() != Some(Ctx::Paren) {
                    return None;
                }
                consumed = true;
            }
            '`' => {
                if stack.last() == Some(&Ctx::Backtick) {
                    stack.pop();
                } else {
                    stack.push(Ctx::Backtick);
                }
                consumed = true;
            }
            '$' if next == Some('{') => {
                stack.push(Ctx::DollarBrace);
                scanner.current.push_str("${");
                i += 2;
                continue;
            }
            '}' if stack.last() == Some(&Ctx::DollarBrace) => {
                stack.pop();
                consumed = true;
            }
            '#' if is_word_start(prev) => return None,
            _ => {}
        }
        if consumed {
            scanner.current.push(c);
            i += 1;
            continue;
        }
        let at_boundary = prev.is_none_or(|p| p.is_whitespace() || p == ';');
        let open_group = c == '{' && at_boundary && next.is_some_and(char::is_whitespace);
        let close_group = c == '}'
            && groups > 0
            && at_boundary
            && next.is_none_or(|n| n.is_whitespace() || matches!(n, ';' | '&' | '|' | ')'));
        if open_group {
            groups += 1;
        }
        if close_group {
            groups -= 1;
        }
        if groups > 0 || close_group {
            scanner.current.push(c);
            i += 1;
            continue;
        }
        if matches!(c, ' ' | '\t')
            && next == Some('-')
            && chars
                .get(i + 2)
                .is_some_and(|n| *n == '-' || n.is_ascii_alphabetic())
            && !scanner.current.trim().is_empty()
            && !scanner.options_ended
        {
            scanner.flags.push(scanner.current.len() + c.len_utf8());
            scanner.options_ended = chars.get(i + 2) == Some(&'-')
                && chars.get(i + 3).is_none_or(|n| n.is_whitespace());
        }
        match c {
            ';' => {
                if next == Some(';') {
                    return None;
                }
                scanner.end_segment(Sep::Semi)?;
            }
            '&' if next == Some('&') => {
                scanner.end_segment(Sep::And)?;
                i += 1;
            }
            '&' if matches!(prev, Some('>' | '<')) || next == Some('>') => scanner.current.push(c),
            '&' => scanner.end_segment(Sep::Background)?,
            '|' if next == Some('|') => {
                scanner.end_segment(Sep::Or)?;
                i += 1;
            }
            '|' if next == Some('&') => return None,
            '|' if prev == Some('>') => scanner.current.push(c),
            '|' => scanner.take_stage()?,
            _ => scanner.current.push(c),
        }
        i += 1;
    }
    if !stack.is_empty() || groups > 0 {
        return None;
    }
    scanner.finish()
}

enum Keyword {
    Header,
    Elif,
    Do,
    Then,
    Else,
    Done,
    Fi,
}

impl Keyword {
    fn parse(word: &str) -> Option<Keyword> {
        match word {
            "for" | "while" | "until" | "select" | "if" => Some(Keyword::Header),
            "elif" => Some(Keyword::Elif),
            "do" => Some(Keyword::Do),
            "then" => Some(Keyword::Then),
            "else" => Some(Keyword::Else),
            "done" => Some(Keyword::Done),
            "fi" => Some(Keyword::Fi),
            _ => None,
        }
    }
}

fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or("")
}

struct Layout {
    lines: Vec<String>,
    stack: Vec<Block>,
    header_open: bool,
    width: usize,
    fence: bool,
}

impl Layout {
    fn body_indent(&self) -> usize {
        INDENT * self.stack.len()
    }

    fn push_line(&mut self, indent: usize, text: &str, sep: Sep) {
        self.lines.push(format!(
            "{}{}{}",
            " ".repeat(indent),
            text,
            sep.suffix(self.fence)
        ));
    }

    fn push_stage(&mut self, indent: usize, prefix: &str, stage: &Stage, sep: Sep) {
        let total = indent
            + prefix.chars().count()
            + stage.text.chars().count()
            + sep.suffix(self.fence).len();
        if total <= self.width || stage.flags.is_empty() {
            self.push_line(indent, &format!("{prefix}{}", stage.text), sep);
            return;
        }
        let mut start = 0;
        for &end in &stage.flags {
            let text = &stage.text[start..end - 1];
            if start == 0 {
                self.push_line(indent, &format!("{prefix}{text} \\"), Sep::None);
            } else {
                self.push_line(indent + INDENT, &format!("{text} \\"), Sep::None);
            }
            start = end;
        }
        self.push_line(indent + INDENT, &stage.text[start..], sep);
    }

    fn statement(&mut self, stages: &[Stage], sep: Sep, chain: bool) {
        let base = if self.header_open && chain {
            INDENT * (self.stack.len() - 1)
        } else {
            self.body_indent()
        };
        let indent = base + if chain { INDENT } else { 0 };
        let joined = join_stages(stages);
        let total = indent + joined.chars().count() + sep.suffix(self.fence).chars().count();
        if stages.len() > 1 && total > self.width {
            for (index, stage) in stages.iter().enumerate() {
                let (indent, prefix) = if index == 0 {
                    (indent, "")
                } else if self.fence {
                    (indent + INDENT, "")
                } else {
                    (indent + INDENT, "| ")
                };
                let suffix = if index + 1 == stages.len() {
                    sep
                } else if self.fence {
                    Sep::Pipe
                } else {
                    Sep::None
                };
                self.push_stage(indent, prefix, stage, suffix);
            }
        } else if stages.len() == 1 {
            self.push_stage(indent, "", &stages[0], sep);
        } else {
            self.push_line(indent, &joined, sep);
        }
        if !chain {
            self.header_open = false;
        }
    }

    fn segment(&mut self, seg: &Segment, prev: Sep) -> Option<()> {
        let mut chain = prev.is_chain();
        let mut stages = seg.stages.clone();
        loop {
            let word = first_word(&stages[0].text).to_string();
            let Some(keyword) = Keyword::parse(&word) else {
                self.statement(&stages, seg.sep, chain);
                return Some(());
            };
            if chain || (prev == Sep::Background && !matches!(keyword, Keyword::Header)) {
                return None;
            }
            let joined = join_stages(&stages);
            match keyword {
                Keyword::Header => {
                    if seg.sep == Sep::Background {
                        return None;
                    }
                    let kind = if word == "if" { Block::If } else { Block::Loop };
                    self.push_line(self.body_indent(), &joined, seg.sep);
                    self.stack.push(kind);
                    self.header_open = true;
                    return Some(());
                }
                Keyword::Elif => {
                    if seg.sep == Sep::Background || self.stack.last() != Some(&Block::If) {
                        return None;
                    }
                    self.push_line(INDENT * (self.stack.len() - 1), &joined, seg.sep);
                    self.header_open = true;
                    return Some(());
                }
                Keyword::Do | Keyword::Then => {
                    let expected = if word == "do" { Block::Loop } else { Block::If };
                    if !self.header_open || self.stack.last() != Some(&expected) {
                        return None;
                    }
                    let joiner = if self.fence { " " } else { "; " };
                    self.lines.last_mut()?.push_str(&format!("{joiner}{word}"));
                    self.header_open = false;
                }
                Keyword::Else => {
                    if self.stack.last() != Some(&Block::If) {
                        return None;
                    }
                    self.push_line(INDENT * (self.stack.len() - 1), "else", Sep::None);
                    self.header_open = false;
                }
                Keyword::Done | Keyword::Fi => {
                    let expected = if word == "done" {
                        Block::Loop
                    } else {
                        Block::If
                    };
                    if self.stack.pop() != Some(expected) || self.header_open {
                        return None;
                    }
                    self.push_line(self.body_indent(), &joined, seg.sep);
                    return Some(());
                }
            }
            let remaining = stages[0].text[word.len()..].trim_start().to_string();
            let removed = stages[0].text.len() - remaining.len();
            stages[0].text = remaining;
            stages[0].flags.retain_mut(|offset| {
                if *offset <= removed {
                    return false;
                }
                *offset -= removed;
                true
            });
            if stages[0].text.is_empty() {
                if stages.len() > 1 {
                    return None;
                }
                if !matches!(seg.sep, Sep::None | Sep::Semi) {
                    return None;
                }
                self.lines.last_mut()?.push_str(seg.sep.suffix(self.fence));
                return Some(());
            }
            chain = false;
        }
    }
}

fn layout(segments: &[Segment], width: usize, fence: bool) -> Option<Vec<String>> {
    let mut state = Layout {
        lines: Vec::new(),
        stack: Vec::new(),
        header_open: false,
        width,
        fence,
    };
    let mut prev = Sep::Semi;
    for segment in segments {
        state.segment(segment, prev)?;
        prev = segment.sep;
    }
    if !state.stack.is_empty() || state.header_open {
        return None;
    }
    Some(state.lines)
}

fn has_sequence(segments: &[Segment]) -> bool {
    segments
        .split_last()
        .is_some_and(|(_, rest)| rest.iter().any(|segment| segment.sep == Sep::Semi))
}

fn format(command: &str, width: usize, fence: bool) -> Option<String> {
    if command.contains(['\n', '\r']) {
        return None;
    }
    let chars: Vec<char> = command.chars().collect();
    let segments = scan(&chars)?;
    if chars.len() <= width && !has_sequence(&segments) {
        return None;
    }
    let lines = layout(&segments, width, fence)?;
    if lines.len() < 2 {
        return None;
    }
    Some(lines.join("\n"))
}

pub fn format_command(command: &str, width: usize) -> Option<String> {
    format(command, width, false)
}

pub fn is_shell_fence(language: Option<&str>) -> bool {
    matches!(language, Some("bash" | "sh" | "zsh" | "shell"))
}

/// Whether a line ends inside a single or double quote, so the quote spans lines.
fn leaves_quote_open(line: &str) -> bool {
    let mut quote = None;
    let mut prev = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '#') if is_word_start(prev) => return false,
            (Some(open), _) if c == open => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None | Some('"'), '\\') => {
                chars.next();
            }
            _ => {}
        }
        prev = Some(c);
    }
    quote.is_some()
}

/// Lay out each line of a shell fence like `format_command`, but keep the block valid shell
/// when pasted: every `;` stays visible and a split pipeline ends its line with ` |`.
///
/// Returns `None` when no line changes, or when any line ends with `\`, opens a heredoc or
/// leaves a quote open.
pub fn format_fence(content: &str, width: usize) -> Option<String> {
    if content.lines().any(|line| {
        line.trim_end().ends_with('\\') || line.contains("<<") || leaves_quote_open(line)
    }) {
        return None;
    }
    let mut changed = false;
    let lines: Vec<String> = content
        .lines()
        .map(|line| {
            let body = line.trim_start();
            let indent = &line[..line.len() - body.len()];
            let Some(formatted) = format(body, width.saturating_sub(indent.chars().count()), true)
            else {
                return line.to_string();
            };
            changed = true;
            formatted
                .lines()
                .map(|formatted_line| format!("{indent}{formatted_line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    changed.then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests;
