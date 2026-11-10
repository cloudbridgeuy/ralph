//! `ralph thread ls` and `/threads`: the thread rows and their table, built
//! from data the shell collected.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadState {
    Open,
    Parked(String),
    Merged(String),
}

impl ThreadState {
    fn rank(&self) -> u8 {
        match self {
            ThreadState::Open => 0,
            ThreadState::Parked(_) => 1,
            ThreadState::Merged(_) => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    pub name: String,
    pub state: ThreadState,
    pub base: usize,
    pub created: String,
    pub active: Option<String>,
    pub messages: usize,
    pub workers: usize,
    pub unread: usize,
}

pub fn sort_rows(rows: &mut [ThreadRow]) {
    rows.sort_by(|a, b| (a.state.rank(), &a.name).cmp(&(b.state.rank(), &b.name)));
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn row(name: &str, state: ThreadState) -> ThreadRow {
        ThreadRow {
            name: name.into(),
            state,
            base: 1,
            created: "2026-01-01T00:00:00Z".into(),
            active: None,
            messages: 0,
            workers: 0,
            unread: 0,
        }
    }

    #[test]
    fn rows_sort_open_then_parked_then_merged_then_name() {
        let at = || "2026-01-02T00:00:00Z".to_string();
        let mut rows = vec![
            row("a-merged", ThreadState::Merged(at())),
            row("z-open", ThreadState::Open),
            row("b-parked", ThreadState::Parked(at())),
            row("a-open", ThreadState::Open),
            row("a-parked", ThreadState::Parked(at())),
        ];
        sort_rows(&mut rows);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["a-open", "z-open", "a-parked", "b-parked", "a-merged"]
        );
    }
}
