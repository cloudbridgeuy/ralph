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
}

impl Layout {
    fn body_indent(&self) -> usize {
        INDENT * self.stack.len()
    }

    fn push_line(&mut self, indent: usize, text: &str, sep: Sep) {
        self.lines
            .push(format!("{}{}{}", " ".repeat(indent), text, sep.suffix()));
    }

    fn push_stage(&mut self, indent: usize, prefix: &str, stage: &Stage, sep: Sep) {
        let total =
            indent + prefix.chars().count() + stage.text.chars().count() + sep.suffix().len();
        if total <= self.width || stage.flags.is_empty() {
            self.push_line(indent, &format!("{prefix}{}", stage.text), sep);
            return;
        }
        let mut start = 0;
        for &end in &stage.flags {
            let text = &stage.text[start..end - 1];
            if start == 0 {
                self.push_line(indent, &format!("{prefix}{text} \\"), Sep::Semi);
            } else {
                self.push_line(indent + INDENT, &format!("{text} \\"), Sep::Semi);
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
        let total = indent + joined.chars().count() + sep.suffix().chars().count();
        if stages.len() > 1 && total > self.width {
            for (index, stage) in stages.iter().enumerate() {
                let (indent, prefix) = if index == 0 {
                    (indent, "")
                } else {
                    (indent + INDENT, "| ")
                };
                let suffix = if index + 1 == stages.len() {
                    sep
                } else {
                    Sep::Semi
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
                    self.lines.last_mut()?.push_str(&format!("; {word}"));
                    self.header_open = false;
                }
                Keyword::Else => {
                    if self.stack.last() != Some(&Block::If) {
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

fn has_sequence(segments: &[Segment]) -> bool {
    segments
        .split_last()
        .is_some_and(|(_, rest)| rest.iter().any(|segment| segment.sep == Sep::Semi))
}

pub fn format_command(command: &str, width: usize) -> Option<String> {
    if command.contains(['\n', '\r']) {
        return None;
    }
    let chars: Vec<char> = command.chars().collect();
    let segments = scan(&chars)?;
    if chars.len() <= width && !has_sequence(&segments) {
        return None;
    }
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
    fn long_gh_command_puts_each_flag_and_value_on_its_own_line() {
        let command = r#"gh run list --branch main --limit 4 --json databaseId,name,status,conclusion,headSha --jq '.[]|"\(.databaseId) \(.name) \(.status) \(.conclusion) \(.headSha[0:7])"'"#;
        let expected = [
            "gh run list \\",
            "  --branch main \\",
            "  --limit 4 \\",
            "  --json databaseId,name,status,conclusion,headSha \\",
            r#"  --jq '.[]|"\(.databaseId) \(.name) \(.status) \(.conclusion) \(.headSha[0:7])"'"#,
        ]
        .join("\n");
        assert_eq!(format_command(command, 80), Some(expected));
    }

    #[test]
    fn flagged_command_wraps_only_above_the_given_width() {
        let command = "gh run list --branch main --limit 4";
        let width = command.chars().count();
        assert_eq!(format_command(command, width + 1), None);
        assert_eq!(format_command(command, width), None);
        assert_eq!(
            format_command(command, width - 1),
            Some("gh run list \\\n  --branch main \\\n  --limit 4".to_string())
        );
    }

    #[test]
    fn quoted_flag_like_values_are_not_split() {
        let command =
            r#"tool --single 'one --not-a-flag two' --double "three --also-not-a-flag four""#;
        let expected = [
            "tool \\",
            "  --single 'one --not-a-flag two' \\",
            r#"  --double "three --also-not-a-flag four""#,
        ]
        .join("\n");
        assert_eq!(format_command(command, 30), Some(expected));
    }

    #[test]
    fn escaped_quotes_keep_flag_like_text_inside_values() {
        for value in [
            r#""one \" --not-a-flag two""#,
            r"$'one\' --not-a-flag \'two'",
            r"\$'one\'",
        ] {
            let command = format!("tool --message {value} --next yes");
            let expected = format!("tool \\\n  --message {value} \\\n  --next yes");
            assert_eq!(format_command(&command, 20), Some(expected));
        }
    }

    #[test]
    fn nested_substitutions_and_groups_are_not_split_at_flags() {
        let command = r#"tool --value "$(echo "a --nested b")" --other $(echo --inner) --path <(cat --inside) --fallback ${value:-a --literal} --tick `echo --hidden`"#;
        let expected = [
            "tool \\",
            r#"  --value "$(echo "a --nested b")" \"#,
            "  --other $(echo --inner) \\",
            "  --path <(cat --inside) \\",
            "  --fallback ${value:-a --literal} \\",
            "  --tick `echo --hidden`",
        ]
        .join("\n");
        assert_eq!(format_command(command, 30), Some(expected));
    }

    #[test]
    fn escaped_spaces_and_dashes_do_not_start_flags() {
        let command = r"tool escaped\ --not-a-flag \--literal --path some\ path --next yes";
        let expected = [
            r"tool escaped\ --not-a-flag \--literal \",
            r"  --path some\ path \",
            "  --next yes",
        ]
        .join("\n");
        assert_eq!(format_command(command, 30), Some(expected));
    }

    #[test]
    fn escaped_trailing_whitespace_in_values_is_preserved() {
        for value in ["value\\ ", "value\\\t"] {
            let command = format!("tool --first {value} --second next");
            let expected = format!("tool \\\n  --first {value} \\\n  --second next");
            assert_eq!(format_command(&command, 20), Some(expected));
        }
    }

    #[test]
    fn short_boolean_and_attached_flags_get_separate_lines() {
        let command = "cargo test --workspace --release -q -j4 --color=always";
        let expected = [
            "cargo test \\",
            "  --workspace \\",
            "  --release \\",
            "  -q \\",
            "  -j4 \\",
            "  --color=always",
        ]
        .join("\n");
        assert_eq!(format_command(command, 30), Some(expected));
    }

    #[test]
    fn negative_values_and_stdin_stay_with_their_flags() {
        let command = "tool --offset -42 --input - --other next";
        let expected = "tool \\\n  --offset -42 \\\n  --input - \\\n  --other next";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn option_terminator_keeps_remaining_arguments_together() {
        let command = "tool --verbose -- --not-a-flag -also-not";
        let expected = "tool \\\n  --verbose \\\n  -- --not-a-flag -also-not";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn unicode_and_leading_whitespace_keep_flag_offsets_correct() {
        let command = "  工具 查找\t--名字 'café --不是选项' --数量 4  ";
        let expected = "工具 查找 \\\n  --名字 'café --不是选项' \\\n  --数量 4";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn wrapped_flags_use_loop_body_indent_after_do() {
        let command = "for f in a; do tool --first value --second other; done";
        let expected = "for f in a; do\n  tool \\\n    --first value \\\n    --second other\ndone";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn wrapped_flags_keep_chain_operators_on_the_last_line() {
        let command = "tool --first value --second other && echo done";
        let expected = "tool \\\n  --first value \\\n  --second other &&\n  echo done";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn wrapped_pipeline_stages_keep_their_pipe_prefix() {
        let command = "tool --first value --second other | next --third value --fourth other";
        let expected = "tool \\\n  --first value \\\n  --second other\n  | next \\\n    --third value \\\n    --fourth other";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn short_statements_in_a_long_command_keep_flags_inline() {
        let command = "tool --one x; tool --two y; tool --three z";
        let expected = "tool --one x\ntool --two y\ntool --three z";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

    #[test]
    fn command_starting_with_a_dash_after_do_is_not_an_option() {
        let command = "for f in a; do -tool --first value --second other; done";
        let expected = "for f in a; do\n  -tool \\\n    --first value \\\n    --second other\ndone";
        assert_eq!(format_command(command, 20), Some(expected.to_string()));
    }

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
            Some("find . \\\n  -name x \\\n  -exec rm {} \\;\necho ok".to_string())
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

    #[test]
    fn short_sequence_splits_on_semicolon() {
        assert_eq!(format_command("a; b", 80), Some("a\nb".to_string()));
    }

    #[test]
    fn short_and_chain_stays_on_one_line() {
        assert_eq!(format_command("cd x && cargo test", 80), None);
    }

    #[test]
    fn short_mixed_chain_splits_at_every_separator() {
        assert_eq!(
            format_command("cd x && make; echo done", 80),
            Some("cd x &&\n  make\necho done".to_string())
        );
    }

    #[test]
    fn short_quoted_or_substituted_semicolon_does_not_split() {
        assert_eq!(format_command("echo 'a; b'", 80), None);
        assert_eq!(format_command("echo $(a; b)", 80), None);
    }

    #[test]
    fn short_trailing_semicolon_adds_no_line() {
        assert_eq!(format_command("echo one;", 80), None);
    }

    #[test]
    fn has_sequence_requires_a_non_final_semicolon() {
        let segments = |command: &str| scan(&command.chars().collect::<Vec<_>>()).unwrap();
        assert!(has_sequence(&segments("a; b")));
        assert!(!has_sequence(&segments("a && b")));
        assert!(!has_sequence(&segments("a;")));
        assert!(!has_sequence(&[]));
    }

    fn assert_lines(command: &str, width: usize, expected: &[&str]) {
        assert_eq!(format_command(command, width), Some(expected.join("\n")));
    }

    #[test]
    fn quotes_nested_in_dollar_brace_and_backtick_stay_in_one_word() {
        let dollar = r#""${a:-"x --yy"}" --bb cc"#;
        assert_lines(dollar, 10, &[r#""${a:-"x --yy"}" \"#, "  --bb cc"]);
        let tick = r#"echo "`x "a --bb"`" --cc d"#;
        assert_lines(tick, 10, &[r#"echo "`x "a --bb"`" \"#, "  --cc d"]);
    }

    #[test]
    fn stray_braces_in_words_do_not_open_groups() {
        assert_lines("echo a{ b; echo c", 10, &["echo a{ b", "echo c"]);
        assert_lines("echo } ; echo x", 80, &["echo }", "echo x"]);
    }

    #[test]
    fn unbalanced_or_unsupported_operators_fall_back() {
        assert_eq!(format_command("a |&>x; echo two", 10), None);
        assert_eq!(format_command("echo one; echo $(abc; echo two", 10), None);
        assert_eq!(format_command("echo x; { echo a; echo b", 10), None);
    }

    #[test]
    fn flagged_stage_wraps_under_its_pipe_or_loop_body() {
        let pipe = ["cat f", "  | grep \\", "    --aa b"];
        assert_lines("cat f | grep --aa b", 12, &pipe);
        let looped = ["for a in 1; do", "  grep \\", "    --aa b", "done"];
        assert_lines("for a in 1; do grep --aa b; done", 12, &looped);
    }

    #[test]
    fn chain_operator_stays_on_the_last_line_of_a_wrapped_statement() {
        let flags = ["grep \\", "  --aa b &&", "  echo x"];
        assert_lines("grep --aa b && echo x", 12, &flags);
        let pipe = ["cat f", "  | wc &&", "  echo x"];
        assert_lines("cat f | wc && echo x", 11, &pipe);
    }

    #[test]
    fn short_pipeline_before_semicolon_stays_on_one_line() {
        assert_lines("cat f | wc; echo x", 10, &["cat f | wc", "echo x"]);
    }

    #[test]
    fn keyword_after_a_chain_or_background_operator_falls_back() {
        let chained = "cd x && for a in 1; do echo $a; done";
        assert_eq!(format_command(chained, 10), None);
        let elif = "if a; then echo b; elif c & echo d; fi";
        assert_eq!(format_command(elif, 10), None);
    }

    #[test]
    fn background_operator_before_a_loop_header_is_allowed() {
        let expected = ["sleep 1 &", "for a in 1; do", "  echo $a", "done"];
        assert_lines("sleep 1 & for a in 1; do echo $a; done", 10, &expected);
    }

    #[test]
    fn misplaced_keywords_in_a_loop_fall_back() {
        let double_do = "for a in 1; do echo x; do echo y; done";
        assert_eq!(format_command(double_do, 10), None);
        let else_in_loop = "for a in 1; do echo x; else echo y; done";
        assert_eq!(format_command(else_in_loop, 10), None);
        assert_eq!(format_command("for a in 1; do | cat; done", 10), None);
        assert_eq!(format_command("for a in 1; do && echo x; done", 10), None);
    }

    #[test]
    fn semicolon_right_after_do_is_accepted() {
        let expected = ["for a in 1; do", "  echo x", "done"];
        assert_lines("for a in 1; do; echo x; done", 10, &expected);
    }
}
