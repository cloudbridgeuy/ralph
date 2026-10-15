const INDENT: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Sep {
    Semi,
    And,
    Or,
    Background,
}

impl Sep {
    fn suffix(self) -> &'static str {
        match self {
            Sep::Semi => "",
            Sep::And => " &&",
            Sep::Or => " ||",
            Sep::Background => " &",
        }
    }

    fn is_chain(self) -> bool {
        matches!(self, Sep::And | Sep::Or)
    }
}

struct Segment {
    stages: Vec<String>,
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
    stages: Vec<String>,
    current: String,
}

impl Scanner {
    fn take_stage(&mut self) -> Option<()> {
        let stage = self.current.trim().to_string();
        self.current.clear();
        if stage.is_empty() {
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
                Sep::Semi | Sep::Background => Some(self.segments),
                Sep::And | Sep::Or => None,
            };
        }
        self.end_segment(Sep::Semi)?;
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
            let close = chars[i + 1..].iter().position(|&x| x == '\'')? + i + 1;
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
        if groups > 0 || open_group || close_group {
            scanner.current.push(c);
            i += 1;
            continue;
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
}

impl Layout {
    fn body_indent(&self) -> usize {
        INDENT * self.stack.len()
    }

    fn push_line(&mut self, indent: usize, text: &str, sep: Sep) {
        self.lines
            .push(format!("{}{}{}", " ".repeat(indent), text, sep.suffix()));
    }

    fn statement(&mut self, stages: &[String], sep: Sep, chain: bool) {
        let base = if self.header_open && chain {
            INDENT * (self.stack.len() - 1)
        } else {
            self.body_indent()
        };
        let indent = base + if chain { INDENT } else { 0 };
        let joined = stages.join(" | ");
        let total = indent + joined.chars().count() + sep.suffix().chars().count();
        if stages.len() > 1 && total > self.width {
            self.push_line(indent, &stages[0], Sep::Semi);
            for stage in &stages[1..] {
                self.push_line(indent + INDENT, &format!("| {stage}"), Sep::Semi);
            }
            if let Some(last) = self.lines.last_mut() {
                last.push_str(sep.suffix());
            }
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
            let word = first_word(&stages[0]).to_string();
            let Some(keyword) = Keyword::parse(&word) else {
                self.statement(&stages, seg.sep, chain);
                return Some(());
            };
            if chain || (prev == Sep::Background && !matches!(keyword, Keyword::Header)) {
                return None;
            }
            let joined = stages.join(" | ");
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
                    if prev != Sep::Semi
                        || !self.header_open
                        || self.stack.last() != Some(&expected)
                    {
                        return None;
                    }
                    self.lines.last_mut()?.push_str(&format!("; {word}"));
                    self.header_open = false;
                }
                Keyword::Else => {
                    if prev != Sep::Semi || self.stack.last() != Some(&Block::If) {
                        return None;
                    }
                    self.push_line(INDENT * (self.stack.len() - 1), "else", Sep::Semi);
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
            stages[0] = stages[0][word.len()..].trim_start().to_string();
            if stages[0].is_empty() {
                if stages.len() > 1 {
                    return None;
                }
                if seg.sep != Sep::Semi {
                    return None;
                }
                return Some(());
            }
            chain = false;
        }
    }
}

fn layout(segments: &[Segment], width: usize) -> Option<Vec<String>> {
    let mut state = Layout {
        lines: Vec::new(),
        stack: Vec::new(),
        header_open: false,
        width,
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

pub fn format_command(command: &str, width: usize) -> Option<String> {
    if command.contains(['\n', '\r']) || command.chars().count() <= width {
        return None;
    }
    let chars: Vec<char> = command.chars().collect();
    let segments = scan(&chars)?;
    let lines = layout(&segments, width)?;
    if lines.len() < 2 {
        return None;
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = "git status --short | head; git log --oneline -1 main; for b in GUZ-450 GUZ-413; do echo \"== $b\"; git log --oneline main..worker/$b; git diff --stat main...worker/$b | tail -1; done; echo \"== overlap\"; comm -12 <(git diff --name-only main...worker/GUZ-450|sort) <(git diff --name-only main...worker/GUZ-413|sort)";

    #[test]
    fn real_example_is_split_per_statement() {
        let expected = [
            "git status --short | head",
            "git log --oneline -1 main",
            "for b in GUZ-450 GUZ-413; do",
            "  echo \"== $b\"",
            "  git log --oneline main..worker/$b",
            "  git diff --stat main...worker/$b | tail -1",
            "done",
            "echo \"== overlap\"",
            "comm -12 <(git diff --name-only main...worker/GUZ-450|sort) <(git diff --name-only main...worker/GUZ-413|sort)",
        ]
        .join("\n");
        assert_eq!(format_command(REAL, 80), Some(expected));
    }

    #[test]
    fn nested_loops_indent_per_level() {
        let cmd = "for a in 1 2; do for b in 3 4; do echo $a$b; done; done";
        let expected = [
            "for a in 1 2; do",
            "  for b in 3 4; do",
            "    echo $a$b",
            "  done",
            "done",
        ]
        .join("\n");
        assert_eq!(format_command(cmd, 20), Some(expected));
    }

    #[test]
    fn if_elif_else_are_handled() {
        let cmd = "if test -f a; then echo a; elif test -f b; then echo b; else echo c; fi";
        let expected = [
            "if test -f a; then",
            "  echo a",
            "elif test -f b; then",
            "  echo b",
            "else",
            "  echo c",
            "fi",
        ]
        .join("\n");
        assert_eq!(format_command(cmd, 20), Some(expected));
    }

    #[test]
    fn while_with_chained_condition_keeps_do_on_last_line() {
        let cmd = "while test -f a && test -f b; do sleep 1; done";
        let expected = ["while test -f a &&", "  test -f b; do", "  sleep 1", "done"].join("\n");
        assert_eq!(format_command(cmd, 20), Some(expected));
    }

    #[test]
    fn and_or_stay_visible_at_line_end() {
        let cmd = "cd some/long/directory && cargo test --workspace || echo failed; echo done";
        let expected = [
            "cd some/long/directory &&",
            "  cargo test --workspace ||",
            "  echo failed",
            "echo done",
        ]
        .join("\n");
        assert_eq!(format_command(cmd, 30), Some(expected));
    }

    #[test]
    fn quoted_semicolon_and_and_are_not_operators() {
        assert_eq!(
            format_command("echo 'a; b && c' \"d; e || f\"; echo two", 20),
            Some("echo 'a; b && c' \"d; e || f\"\necho two".to_string())
        );
    }

    #[test]
    fn escaped_semicolon_is_not_an_operator() {
        assert_eq!(
            format_command("find . -name x -exec rm {} \\; ; echo ok", 20),
            Some("find . -name x -exec rm {} \\;\necho ok".to_string())
        );
    }

    #[test]
    fn nested_constructs_are_not_split() {
        let cmd = "echo $(a; b) `c; d` ${x:-y;z} <(e; f) >(g; h) ( i; j ) { k; l; }; echo next";
        let expected =
            "echo $(a; b) `c; d` ${x:-y;z} <(e; f) >(g; h) ( i; j ) { k; l; }\necho next";
        assert_eq!(format_command(cmd, 20), Some(expected.to_string()));
    }

    #[test]
    fn quotes_nested_in_command_substitution_are_respected() {
        let cmd = "echo \"$(echo \"a;b\")\"; echo two";
        assert_eq!(
            format_command(cmd, 10),
            Some("echo \"$(echo \"a;b\")\"\necho two".to_string())
        );
    }

    #[test]
    fn long_pipeline_breaks_before_pipes() {
        let cmd = "cat some/file.txt | grep pattern | sort -u | head -n 5";
        let expected = [
            "cat some/file.txt",
            "  | grep pattern",
            "  | sort -u",
            "  | head -n 5",
        ]
        .join("\n");
        assert_eq!(format_command(cmd, 30), Some(expected));
    }

    #[test]
    fn pipeline_break_in_loop_body_uses_body_indent() {
        let cmd = "for f in a; do cat some/file.txt | grep pattern | sort -u; done";
        let expected = [
            "for f in a; do",
            "  cat some/file.txt",
            "    | grep pattern",
            "    | sort -u",
            "done",
        ]
        .join("\n");
        assert_eq!(format_command(cmd, 30), Some(expected));
    }

    #[test]
    fn redirects_with_ampersand_and_pipe_are_not_operators() {
        assert_eq!(
            format_command("cmd 2>&1 &> out >| f; echo two", 10),
            Some("cmd 2>&1 &> out >| f\necho two".to_string())
        );
    }

    #[test]
    fn background_ampersand_ends_line() {
        assert_eq!(
            format_command("sleep 10 & echo started", 10),
            Some("sleep 10 &\necho started".to_string())
        );
    }

    #[test]
    fn trailing_semicolon_is_dropped() {
        assert_eq!(
            format_command("echo one; echo two;", 10),
            Some("echo one\necho two".to_string())
        );
    }

    #[test]
    fn short_command_is_unchanged() {
        assert_eq!(format_command("cd x && cargo test", 80), None);
    }

    #[test]
    fn long_single_command_is_unchanged() {
        assert_eq!(
            format_command("echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 10),
            None
        );
    }

    #[test]
    fn multiline_message_stays_intact() {
        let cmd = "git commit -m \"title\n\nbody; more\" && git push";
        assert_eq!(format_command(cmd, 10), None);
    }

    #[test]
    fn heredoc_stays_intact() {
        let cmd = "cat <<EOF > f; echo x\nbody; here\nEOF";
        assert_eq!(format_command(cmd, 10), None);
    }

    #[test]
    fn unbalanced_input_falls_back() {
        assert_eq!(format_command("echo 'abc; echo two; echo three", 10), None);
        assert_eq!(format_command("echo \"abc; echo two; echo three", 10), None);
        assert_eq!(format_command("echo $(abc; echo two; echo three", 10), None);
        assert_eq!(format_command("echo abc); echo two; echo three", 10), None);
        assert_eq!(format_command("echo ${abc; echo two; echo three", 10), None);
    }

    #[test]
    fn unsupported_constructs_fall_back() {
        assert_eq!(
            format_command("case x in a) echo a;; esac; echo two", 10),
            None
        );
        assert_eq!(format_command("echo one; # comment; echo two", 10), None);
        assert_eq!(format_command("for a in 1; do echo $a; echo two", 10), None);
        assert_eq!(format_command("echo one; done; echo two", 10), None);
        assert_eq!(format_command("echo one && ; echo two", 10), None);
        assert_eq!(format_command("echo one; echo two &&", 10), None);
        assert_eq!(format_command("a |& b; echo two", 10), None);
    }

    #[test]
    fn hash_inside_word_is_not_a_comment() {
        assert_eq!(
            format_command("echo ${#x} $# a#b; echo two", 10),
            Some("echo ${#x} $# a#b\necho two".to_string())
        );
    }
}
