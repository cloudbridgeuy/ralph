use super::lines::Statement;
use super::Refusal;

const MAX_JOURNEY_TASKS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JourneyLine {
    Title,
    Section,
    Task,
    Unknown,
}

impl JourneyLine {
    fn parse(statement: Statement<'_>) -> Self {
        if statement.has_keyword("title") {
            Self::Title
        } else if statement.has_keyword("section") {
            Self::Section
        } else if statement.code.contains(':') {
            Self::Task
        } else {
            Self::Unknown
        }
    }
}

struct JourneySize {
    tasks: usize,
}

fn size_of(lines: &[JourneyLine]) -> JourneySize {
    JourneySize {
        tasks: lines.iter().filter(|&&l| l == JourneyLine::Task).count(),
    }
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<JourneyLine> = body.iter().copied().map(JourneyLine::parse).collect();
    if lines.contains(&JourneyLine::Unknown) {
        return Err(Refusal::Unsupported);
    }
    if size_of(&lines).tasks > MAX_JOURNEY_TASKS {
        return Err(Refusal::TooManyTasks);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn journey_draws() {
        assert_draws(
            "journey\n  title My day\n  section Work\n    Make tea: 5: Me, Cat",
            DiagramKind::Journey,
        );
    }

    #[test]
    fn journey_refuses_unknown_lines() {
        assert_unsupported(&["journey\n  just words"]);
    }

    #[test]
    fn journey_task_limit_ignores_titles_and_sections() {
        let journey = |n| {
            let tasks = repeated("section S: x", n, |i| format!("t{i}: 3: Me"));
            format!("journey\n  Title a: b\n  {tasks}")
        };
        assert_draws(&journey(100), DiagramKind::Journey);
        assert_eq!(verdict(&journey(101)), Err(Refusal::TooManyTasks));
    }
}
