use crate::chat_help::Command;
use crate::chat_hold::{word_completion, HoldCompletion};
use crate::chat_memory::Mode;
use crate::chat_send_it::SendItSub;
use crate::chat_stage::{Kind, StageVerb};
use crate::chat_status::{HistoryFlag, InboxSub, WorkersSub};
use crate::chat_thread::ThreadName;
use crate::chat_thread_cmd::{MergeFlag, OpenFlag, ThreadSub};
use crate::chat_thread_model::THREAD_MODEL_ALIASES;
use crate::diff_command::{is_worker_id, DiffWord};
use crate::schedule_command::ScheduleSub;
use crate::watch_command::{WatchFlag, WatchSub};
use crate::worker_costs::views::View;
use crate::worker_costs::window::Window;
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
    pub workers: &'a [String],
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
        Command::Inbox => first_word::<InboxSub>(args),
        Command::Watch => watch_offers(args),
        Command::Schedule => first_word::<ScheduleSub>(args),
        Command::SendIt => first_word::<SendItSub>(args),
        Command::Stage => stage_offers(args),
        Command::Diff => diff_offers(args, facts),
        Command::Workers => workers_offers(args, facts),
        Command::Answer if args.is_empty() => Offers::names(facts.workers.iter().cloned()),
        Command::Read => read_offers(args),
        Command::Search if !args.is_empty() => cursor_offers(args),
        Command::Rules if args.is_empty() => every::<HistoryFlag>(),
        Command::Help
        | Command::Hold
        | Command::Nudge
        | Command::Config
        | Command::View
        | Command::Rerender
        | Command::Answer
        | Command::Zoom
        | Command::Recall
        | Command::Search
        | Command::Rules => Offers::default(),
    }
}

fn every<K: Keyword>() -> Offers {
    Offers::keywords(K::ALL.iter().copied().map(K::word))
}

fn first_word<K: Keyword>(args: &[&str]) -> Offers {
    match args {
        [] => every::<K>(),
        _ => Offers::default(),
    }
}

fn watch_offers(args: &[&str]) -> Offers {
    let [word] = args else {
        return first_word::<WatchSub>(args);
    };
    match WatchSub::parse(word) {
        Some(WatchSub::Start) => every::<WatchFlag>(),
        Some(WatchSub::List | WatchSub::Stop) | None => Offers::default(),
    }
}

fn stage_offers(args: &[&str]) -> Offers {
    let [word] = args else {
        return first_word::<StageVerb>(args);
    };
    match StageVerb::parse(word) {
        Some(StageVerb::Add) => every::<Kind>(),
        Some(StageVerb::Done | StageVerb::Edit) | None => Offers::default(),
    }
}

fn diff_offers(args: &[&str], facts: &CompletionFacts) -> Offers {
    let Some((first, rest)) = args.split_first() else {
        return every::<DiffWord>();
    };
    match (DiffWord::parse(first), rest) {
        (Some(DiffWord::Worker), []) => Offers::names(facts.workers.iter().cloned()),
        (Some(DiffWord::Worker), [id]) if is_worker_id(id) => {
            let windows = DiffWord::ALL
                .iter()
                .copied()
                .filter(|word| word.window().is_some());
            Offers::keywords(windows.map(Keyword::word))
        }
        (Some(DiffWord::Worker | DiffWord::Now | DiffWord::Last | DiffWord::Ago) | None, _) => {
            Offers::default()
        }
    }
}

fn workers_offers(args: &[&str], facts: &CompletionFacts) -> Offers {
    let Some((first, rest)) = args.split_first() else {
        let mut offers = every::<WorkersSub>();
        offers.names.extend(facts.workers.iter().cloned());
        return offers;
    };
    match WorkersSub::parse(first) {
        Some(WorkersSub::Costs) => costs_offers(rest),
        None => Offers::default(),
    }
}

fn costs_offers(args: &[&str]) -> Offers {
    match args {
        [] => {
            let mut offers = every::<View>();
            offers.keywords.extend(every::<Window>().keywords);
            offers
        }
        [view] if View::parse(view).is_some() => every::<Window>(),
        _ => Offers::default(),
    }
}

fn read_offers(args: &[&str]) -> Offers {
    match args {
        [id] if HistoryFlag::parse(id).is_none() => {
            let mut offers = every::<Mode>();
            offers.keywords.extend(every::<HistoryFlag>().keywords);
            offers
        }
        [_, mode] if Mode::parse(mode).is_some() => cursor_offers(args),
        _ => Offers::default(),
    }
}

fn cursor_offers(args: &[&str]) -> Offers {
    if args.iter().any(|word| HistoryFlag::parse(word).is_some()) {
        Offers::default()
    } else {
        every::<HistoryFlag>()
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
