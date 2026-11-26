use super::lines::Statement;
use super::Refusal;

const MAX_PERIODS: usize = 100;
const MAX_EVENTS: usize = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventCount(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineLine {
    Title,
    Section,
    Accessibility,
    Period(EventCount),
    Unknown,
}

impl TimelineLine {
    fn parse(statement: Statement<'_>) -> Self {
        if statement.is_accessibility() {
            return Self::Accessibility;
        }
        if statement.has_keyword("title") {
            return Self::Title;
        }
        if statement.has_keyword("section") {
            return Self::Section;
        }
        match statement.code.split_once(':') {
            Some((period, events)) if !period.is_empty() => {
                let named = events.split(':').filter(|e| !e.trim().is_empty());
                Self::Period(EventCount(named.count()))
            }
            _ => Self::Unknown,
        }
    }
}

struct TimelineSize {
    periods: usize,
    events: usize,
}

fn size_of(lines: &[TimelineLine]) -> TimelineSize {
    let mut size = TimelineSize {
        periods: 0,
        events: 0,
    };
    for line in lines {
        if let TimelineLine::Period(EventCount(events)) = line {
            size.periods += 1;
            size.events += events;
        }
    }
    size
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<TimelineLine> = body.iter().copied().map(TimelineLine::parse).collect();
    if lines.contains(&TimelineLine::Unknown) {
        return Err(Refusal::Unsupported);
    }
    let size = size_of(&lines);
    if size.periods > MAX_PERIODS || size.events > MAX_EVENTS {
        return Err(Refusal::TooManyEntries);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn timeline_draws() {
        assert_draws(
            "timeline\n  Title History\n  accTitle: x\n  section Early\n    2002 : LinkedIn : MySpace",
            DiagramKind::Timeline,
        );
    }

    #[test]
    fn timeline_refuses_lines_the_renderer_drops() {
        assert_unsupported(&[
            "timeline\n  2002 : a\n       : b",
            "timeline\n  2002 : a\n  2003",
        ]);
    }

    #[test]
    fn timeline_period_limit() {
        let timeline = |n| repeated("timeline", n, |i| format!("p{i} : e"));
        assert_draws(&timeline(100), DiagramKind::Timeline);
        assert_eq!(verdict(&timeline(101)), Err(Refusal::TooManyEntries));
    }

    #[test]
    fn timeline_event_limit_counts_every_event() {
        let timeline = |extra: &str| {
            let rows = repeated("timeline", 60, |i| format!("p{i} : a : b : c : d : e : "));
            format!("{rows}{extra}")
        };
        assert_draws(&timeline(""), DiagramKind::Timeline);
        assert_eq!(verdict(&timeline("  q : f")), Err(Refusal::TooManyEntries));
    }
}
