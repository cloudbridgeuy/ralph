use super::lines::Statement;
use super::Refusal;

const MAX_SLICES: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PieLine {
    Slice,
    Unknown,
}

impl PieLine {
    fn parse(statement: Statement<'_>) -> Self {
        if statement.code.starts_with('"') {
            Self::Slice
        } else {
            Self::Unknown
        }
    }
}

struct PieSize {
    slices: usize,
}

fn size_of(lines: &[PieLine]) -> PieSize {
    PieSize {
        slices: lines.iter().filter(|&&l| l == PieLine::Slice).count(),
    }
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<PieLine> = body.iter().copied().map(PieLine::parse).collect();
    if lines.contains(&PieLine::Unknown) {
        return Err(Refusal::Unsupported);
    }
    if size_of(&lines).slices > MAX_SLICES {
        return Err(Refusal::TooManySlices);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, repeated, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn pie_draws() {
        assert_draws(
            "pie showData title Pets\n  %% note\n  \"Dogs\" : 3\n  \"Cats\" : 1 %% few",
            DiagramKind::Pie,
        );
    }

    #[test]
    fn pie_refuses_lines_that_are_not_slices() {
        assert_unsupported(&[
            "pie\n  title Pets\n  \"Dogs\" : 3",
            "pie\n  accTitle: x\n  \"Dogs\" : 3",
        ]);
    }

    #[test]
    fn pie_slice_limit() {
        let pie = |n| repeated("pie", n, |i| format!("\"s{i}\" : 1"));
        assert_draws(&pie(30), DiagramKind::Pie);
        assert_eq!(verdict(&pie(31)), Err(Refusal::TooManySlices));
    }
}
