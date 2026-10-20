#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Token {
    pub(super) word: String,
    pub(super) parts: Vec<String>,
    pub(super) start: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_compound(word: &str) -> bool {
    word.chars().any(|c| !is_word(c))
}

fn parts_of(word: &str) -> Vec<String> {
    if !is_compound(word) {
        return Vec::new();
    }
    word.split(|c| !is_word(c))
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn joiner_len(chars: &[(usize, char)], at: usize) -> usize {
    let next_is_word = |k: usize| chars.get(k).is_some_and(|(_, c)| is_word(*c));
    match chars.get(at).map(|(_, c)| *c) {
        Some('-' | '.') if next_is_word(at + 1) => 1,
        Some(':') if chars.get(at + 1).is_some_and(|(_, c)| *c == ':') && next_is_word(at + 2) => 2,
        _ => 0,
    }
}

pub(super) fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !is_word(chars[i].1) {
            i += 1;
            continue;
        }
        let mut j = i;
        loop {
            while j < chars.len() && is_word(chars[j].1) {
                j += 1;
            }
            match joiner_len(&chars, j) {
                0 => break,
                joiner => j += joiner,
            }
        }
        let start = chars[i].0;
        let end = chars.get(j).map_or(text.len(), |(at, _)| *at);
        let word = text[start..end].to_lowercase();
        tokens.push(Token {
            parts: parts_of(&word),
            word,
            start,
        });
        i = j;
    }
    tokens
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Unit {
    pub(super) words: Vec<String>,
    pub(super) required: bool,
}

impl Unit {
    pub(super) fn normalized(&self) -> String {
        let joined = self.words.join(" ");
        if self.required {
            format!("\"{joined}\"")
        } else {
            joined
        }
    }

    pub(super) fn matches_at(&self, tokens: &[Token], at: usize) -> bool {
        if let [word] = self.words.as_slice() {
            if !is_compound(word) && !self.required {
                return tokens[at].word == *word || tokens[at].parts.contains(word);
            }
        }
        self.words
            .iter()
            .zip(&tokens[at..])
            .all(|(word, token)| *word == token.word)
    }

    pub(super) fn occurrences(&self, tokens: &[Token]) -> (u32, Option<usize>) {
        let width = self.words.len();
        if tokens.len() < width {
            return (0, None);
        }
        let mut found = (0..=tokens.len() - width).filter(|at| self.matches_at(tokens, *at));
        let first = found.next();
        let count = u32::from(first.is_some()) + found.count() as u32;
        (count, first.map(|at| tokens[at].start))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Query {
    pub(super) units: Vec<Unit>,
}

impl Query {
    pub(super) fn parse(text: &str) -> Result<Query, String> {
        if text.matches('"').count() % 2 == 1 {
            return Err("unclosed quote in query.".to_string());
        }
        let mut units: Vec<Unit> = Vec::new();
        for (i, segment) in text.split('"').enumerate() {
            let required = i % 2 == 1;
            let words: Vec<String> = tokenize(segment).into_iter().map(|t| t.word).collect();
            let found: Vec<Unit> = if required {
                Some(words)
                    .filter(|w| !w.is_empty())
                    .into_iter()
                    .map(|words| Unit { words, required })
                    .collect()
            } else {
                words
                    .into_iter()
                    .map(|word| Unit {
                        words: vec![word],
                        required,
                    })
                    .collect()
            };
            for unit in found {
                match units.iter_mut().find(|u| u.words == unit.words) {
                    Some(existing) => existing.required |= unit.required,
                    None => units.push(unit),
                }
            }
        }
        if units.is_empty() {
            return Err("query has no searchable words.".to_string());
        }
        Ok(Query { units })
    }

    pub(super) fn normalized(&self) -> String {
        self.units
            .iter()
            .map(Unit::normalized)
            .collect::<Vec<_>>()
            .join(" ")
    }
}
