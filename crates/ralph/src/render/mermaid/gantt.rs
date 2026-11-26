use super::lines::Statement;
use super::Refusal;

const MAX_GANTT_TASKS: usize = 100;
const MAX_TASK_LENGTH: Days = Days(3650);
const STATUS_TAGS: [&str; 4] = ["done", "active", "crit", "milestone"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Days(u32);

impl Days {
    fn parse(raw: &str) -> Option<Self> {
        let (number, unit) = raw.split_at_checked(raw.len().checked_sub(1)?)?;
        let number: u64 = number.parse().ok().filter(|&n| n > 0)?;
        let days = match unit {
            "d" => number,
            "w" => number.saturating_mul(7),
            "h" => number.div_ceil(24),
            _ => return None,
        };
        Some(Self(u32::try_from(days).unwrap_or(u32::MAX)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    Date,
    After,
    Implicit,
}

impl Start {
    fn explicit(field: &str) -> Option<Self> {
        if is_date_shaped(field) {
            Some(Self::Date)
        } else if field.starts_with("after ") || field.starts_with("After ") {
            Some(Self::After)
        } else {
            None
        }
    }
}

fn is_date_shaped(field: &str) -> bool {
    field.len() == 10
        && field.bytes().enumerate().all(|(at, b)| match at {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GanttLine {
    Title,
    Setting,
    Section,
    Task { start: Start, days: Days },
    Calendar,
    Unknown,
}

impl GanttLine {
    fn parse(statement: Statement<'_>) -> Self {
        let first_word = statement
            .code
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_lowercase();
        if statement.has_keyword("title") {
            return Self::Title;
        }
        match first_word.as_str() {
            "excludes" | "includes" => Self::Calendar,
            "tickinterval" | "weekday" | "click" | "todaymarker" => Self::Setting,
            _ if statement.has_keyword("dateFormat") => Self::Setting,
            _ if statement.has_keyword("axisFormat") => Self::Setting,
            _ if statement.has_keyword("section") => Self::Section,
            _ => Self::task(statement.code).unwrap_or(Self::Unknown),
        }
    }

    fn task(code: &str) -> Option<Self> {
        let (name, spec) = code.split_once(':')?;
        if name.trim().is_empty() {
            return None;
        }
        let fields: Vec<&str> = spec
            .split(',')
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .filter(|field| !STATUS_TAGS.contains(&field.to_lowercase().as_str()))
            .collect();
        let (length, prefix) = fields.split_last()?;
        let start = match prefix {
            [] => Start::Implicit,
            [single] => Start::explicit(single).unwrap_or(Start::Implicit),
            [_, second] => Start::explicit(second)?,
            _ => return None,
        };
        Some(Self::Task {
            start,
            days: Days::parse(length)?,
        })
    }
}

struct GanttSize {
    tasks: usize,
    longest: Days,
    unanchored_sections: usize,
}

fn size_of(lines: &[GanttLine]) -> GanttSize {
    let mut size = GanttSize {
        tasks: 0,
        longest: Days(0),
        unanchored_sections: 0,
    };
    let mut opens_section = true;
    for line in lines {
        match *line {
            GanttLine::Section => opens_section = true,
            GanttLine::Task { start, days } => {
                size.tasks += 1;
                size.longest = size.longest.max(days);
                size.unanchored_sections += usize::from(opens_section && start == Start::Implicit);
                opens_section = false;
            }
            GanttLine::Title | GanttLine::Setting | GanttLine::Calendar | GanttLine::Unknown => {}
        }
    }
    size
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<GanttLine> = body.iter().copied().map(GanttLine::parse).collect();
    let size = size_of(&lines);
    if lines.contains(&GanttLine::Calendar)
        || lines.contains(&GanttLine::Unknown)
        || size.unanchored_sections > 0
    {
        return Err(Refusal::Unsupported);
    }
    if size.tasks > MAX_GANTT_TASKS {
        return Err(Refusal::TooManyTasks);
    }
    if size.longest > MAX_TASK_LENGTH {
        return Err(Refusal::TooLongTask);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn gantt_draws() {
        assert_draws(
            "gantt\n  Title Plan\n  dateFormat YYYY-MM-DD\n  axisFormat %m\n  todayMarker off\n  section A\n    Design :done, d1, 2024-01-01, 5d\n    Build :after d1, 2w\n    Ship :crit, 3d\n  section B\n    Test :t1, after d1, 48h",
            DiagramKind::Gantt,
        );
    }

    #[test]
    fn gantt_refuses_calendars_and_tasks_that_start_today() {
        assert_unsupported(&[
            "gantt\n  excludes weekends\n  A :2024-01-01, 1d",
            "gantt\n  Includes 2024-01-06\n  A :2024-01-01, 1d",
            "gantt\n  A :3d",
            "gantt\n  A :2024-01-01, 1d\n  section S\n  B :b1, 2d",
        ]);
    }

    #[test]
    fn gantt_refuses_lines_it_cannot_measure() {
        assert_unsupported(&[
            "gantt\n  junk",
            "gantt\n  accTitle: x\n  A :2024-01-01, 1d",
            "gantt\n  A :2024-01-01, 0d",
            "gantt\n  A :2024-01-01, 1y",
            "gantt\n  A :a, b, 1d",
            "gantt\n  A :a, 2024-01-01, b, 1d",
            "gantt\n  :2024-01-01, 1d",
        ]);
    }

    #[test]
    fn gantt_task_limit() {
        let gantt = |n| repeated("gantt", n, |i| format!("t{i} :2024-01-01, 1d"));
        assert_draws(&gantt(100), DiagramKind::Gantt);
        assert_eq!(verdict(&gantt(101)), Err(Refusal::TooManyTasks));
    }

    #[test]
    fn gantt_duration_limit_in_days_covers_every_unit() {
        let gantt = |length: &str| format!("gantt\n  A :2024-01-01, {length}");
        for length in ["3650d", "521w", "87600h"] {
            assert_draws(&gantt(length), DiagramKind::Gantt);
        }
        for length in ["3651d", "522w", "87601h", "9000000000000000000w"] {
            assert_eq!(
                verdict(&gantt(length)),
                Err(Refusal::TooLongTask),
                "{length}"
            );
        }
    }
}
