use super::lines::{body_lines, strip_inline_comment};
use super::Refusal;
use std::collections::HashSet;

const MAX_GIT_COMMITS: usize = 200;
const MAX_GIT_BRANCHES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BranchName<'a>(&'a str);

impl<'a> BranchName<'a> {
    const MAIN: Self = Self("main");

    fn parse(raw: &'a str) -> Option<Self> {
        let name = raw.trim();
        (!name.is_empty()).then_some(Self(name))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitLine<'a> {
    Commit,
    Branch(BranchName<'a>),
    Switch,
    Other,
}

impl<'a> GitLine<'a> {
    fn parse(line: &'a str) -> Self {
        let code = strip_inline_comment(line);
        let (verb, rest) = code.split_once(char::is_whitespace).unwrap_or((code, ""));
        match verb {
            "commit" | "merge" | "cherry-pick" => Self::Commit,
            "branch" => BranchName::parse(rest).map_or(Self::Other, Self::Branch),
            "switch" => Self::Switch,
            _ => Self::Other,
        }
    }
}

struct GitSize {
    commits: usize,
    branches: usize,
}

fn size_of(lines: &[GitLine<'_>]) -> GitSize {
    let mut commits = 0;
    let mut branches = HashSet::from([BranchName::MAIN]);
    for line in lines {
        match line {
            GitLine::Commit => commits += 1,
            GitLine::Branch(name) => {
                branches.insert(*name);
            }
            GitLine::Switch | GitLine::Other => {}
        }
    }
    GitSize {
        commits,
        branches: branches.len(),
    }
}

pub(super) fn check(source: &str) -> Result<(), Refusal> {
    let lines: Vec<GitLine<'_>> = body_lines(source).skip(1).map(GitLine::parse).collect();
    if lines.contains(&GitLine::Switch) {
        return Err(Refusal::Unsupported);
    }
    let size = size_of(&lines);
    if size.commits > MAX_GIT_COMMITS {
        return Err(Refusal::TooManyCommits);
    }
    if size.branches > MAX_GIT_BRANCHES {
        return Err(Refusal::TooManyBranches);
    }
    Ok(())
}
