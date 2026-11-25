use crate::chat_help::Command;
use crate::chat_hold::{word_completion, HoldCompletion};
use crate::chat_thread::ThreadName;
use crate::chat_thread_cmd::{MergeFlag, OpenFlag, ThreadSub};
use crate::chat_thread_model::THREAD_MODEL_ALIASES;
use std::collections::BTreeSet;
use std::ops::Range;

pub trait Keyword: Copy + 'static {
    const ALL: &'static [Self];

    fn word(self) -> &'static str;

    fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|keyword| keyword.word() == word)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecCompletion {
    pub span: Range<usize>,
    pub values: Vec<SpecValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecValue {
    pub text: String,
    pub append_space: bool,
}

impl From<HoldCompletion> for SpecCompletion {
    fn from(found: HoldCompletion) -> Self {
        SpecCompletion {
            span: found.span,
            values: found
                .values
                .into_iter()
                .map(|text| SpecValue {
                    text,
                    append_space: false,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompletionFacts<'a> {
    pub current: Option<&'a ThreadName>,
    pub threads: &'a [String],
}

#[derive(Default)]
struct Offers {
    keywords: BTreeSet<String>,
    names: BTreeSet<String>,
}

impl Offers {
    fn keywords<'a>(words: impl IntoIterator<Item = &'a str>) -> Self {
        Offers {
            keywords: words.into_iter().map(str::to_string).collect(),
            ..Offers::default()
        }
    }

    fn names(names: impl IntoIterator<Item = String>) -> Self {
        Offers {
            names: names.into_iter().collect(),
            ..Offers::default()
        }
    }
}

pub fn spec_completions(
    line: &str,
    cursor: usize,
    facts: &CompletionFacts,
) -> Option<SpecCompletion> {
    let before = line.get(..cursor)?.trim_start();
    if !before.starts_with('/') {
        return None;
    }
    let partial = before
        .rsplit(char::is_whitespace)
        .next()
        .unwrap_or_default();
    let typed: Vec<&str> = before[..before.len() - partial.len()]
        .split_whitespace()
        .collect();
    let offers = match typed.split_first() {
        None => Offers::keywords(Command::ALL.map(Command::name)),
        Some((head, args)) => {
            let command = Command::ALL
                .into_iter()
                .find(|command| command.name() == *head)?;
            command_offers(command, args, partial, facts)
        }
    };
    let targets = offers.keywords.union(&offers.names).cloned().collect();
    let found = word_completion(partial, cursor, targets);
    let spaced = !line.get(cursor..)?.starts_with(char::is_whitespace);
    (!found.values.is_empty()).then(|| SpecCompletion {
        span: found.span,
        values: found
            .values
            .into_iter()
            .map(|text| SpecValue {
                append_space: spaced && offers.keywords.contains(&text),
                text,
            })
            .collect(),
    })
}

fn command_offers(
    command: Command,
    args: &[&str],
    partial: &str,
    facts: &CompletionFacts,
) -> Offers {
    match command {
        Command::Thread => thread_offers(args, partial, facts),
        Command::Help
        | Command::Inbox
        | Command::Watch
        | Command::Schedule
        | Command::Diff
        | Command::Workers
        | Command::Hold
        | Command::Nudge
        | Command::SendIt
        | Command::Stage
        | Command::Config
        | Command::View
        | Command::Rerender
        | Command::Answer
        | Command::Read
        | Command::Zoom
        | Command::Recall
        | Command::Search
        | Command::Rules => Offers::default(),
    }
}

fn thread_offers(args: &[&str], partial: &str, facts: &CompletionFacts) -> Offers {
    let Some((first, rest)) = args.split_first() else {
        let subs = ThreadSub::ALL
            .iter()
            .copied()
            .filter(|sub| facts.current.is_some() || *sub != ThreadSub::Main);
        return Offers::keywords(subs.map(Keyword::word));
    };
    match ThreadSub::parse(first) {
        Some(ThreadSub::Switch) if rest.is_empty() => {
            let current = facts.current.map(ThreadName::as_str);
            let others = facts
                .threads
                .iter()
                .filter(|name| Some(name.as_str()) != current);
            Offers::names(others.cloned())
        }
        Some(ThreadSub::Open) => open_offers(rest, partial),
        Some(ThreadSub::Merge) => merge_offers(rest, partial, facts),
        Some(ThreadSub::Drop | ThreadSub::Park | ThreadSub::Pull)
            if rest.is_empty() && facts.current.is_none() =>
        {
            Offers::names(facts.threads.iter().cloned())
        }
        Some(
            ThreadSub::Switch
            | ThreadSub::Drop
            | ThreadSub::Ls
            | ThreadSub::Main
            | ThreadSub::Park
            | ThreadSub::Pull
            | ThreadSub::Revive,
        )
        | None => Offers::default(),
    }
}

fn open_offers(args: &[&str], partial: &str) -> Offers {
    if args.last().and_then(|word| OpenFlag::parse(word)) == Some(OpenFlag::Model) {
        return Offers::keywords(THREAD_MODEL_ALIASES);
    }
    let mut used = Vec::new();
    let mut names = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        match OpenFlag::parse(word) {
            Some(flag) => {
                used.push(flag);
                match flag {
                    OpenFlag::Model => {
                        words.next();
                    }
                    OpenFlag::AutoMerge | OpenFlag::AutoDrop => {}
                }
            }
            None => names.push(*word),
        }
    }
    if !takes_flag(&names, partial) {
        return Offers::default();
    }
    let unused = OpenFlag::ALL
        .iter()
        .copied()
        .filter(|flag| !used.iter().any(|used| flag.excludes(*used)));
    Offers::keywords(unused.map(Keyword::word))
}

fn merge_offers(args: &[&str], partial: &str, facts: &CompletionFacts) -> Offers {
    let names: Vec<&str> = args
        .iter()
        .copied()
        .filter(|word| MergeFlag::typed(word).is_none())
        .collect();
    let takes_view = names.len() == args.len() && takes_flag(&names, partial);
    let flags = MergeFlag::ALL.iter().copied().filter(|_| takes_view);
    let mut offers = Offers::keywords(flags.map(Keyword::word));
    if names.is_empty() && facts.current.is_none() {
        offers.names.extend(facts.threads.iter().cloned());
    }
    offers
}

fn takes_flag(names: &[&str], partial: &str) -> bool {
    match names {
        [] => partial.starts_with('-'),
        [name] => !name.starts_with("--"),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
