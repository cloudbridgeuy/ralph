use super::lines::{strip_inline_comment, Statement};
use super::Refusal;

const MAX_SERIES_VALUES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct Value(f64);

impl Value {
    fn parse(raw: &str) -> Option<Self> {
        let number: f64 = raw.trim().parse().ok()?;
        number.is_finite().then_some(Self(number))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Range {
    min: Value,
    max: Value,
}

impl Range {
    fn parse_axis(raw: &str) -> Option<Self> {
        let bounds = match raw.strip_prefix('"') {
            Some(label) => label.split_once('"')?.1,
            None => raw,
        };
        let (min, max) = bounds.trim().split_once(" --> ")?;
        let range = Self {
            min: Value::parse(min)?,
            max: Value::parse(max)?,
        };
        (range.min < range.max).then_some(range)
    }

    fn including(self, other: Self) -> Self {
        Self {
            min: Value(self.min.0.min(other.min.0)),
            max: Value(self.max.0.max(other.max.0)),
        }
    }

    fn contains(self, other: Self) -> bool {
        self.min <= other.min && other.max <= self.max
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Series {
    Bar,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum XyLine {
    Title,
    XAxis,
    YAxis(Range),
    Data {
        series: Series,
        count: usize,
        extent: Range,
    },
    Accessibility,
    Unknown,
}

impl XyLine {
    fn parse(statement: Statement<'_>) -> Self {
        if statement.is_accessibility() {
            return Self::Accessibility;
        }
        let parsed = statement
            .code
            .split_once(' ')
            .and_then(|(keyword, rest)| match keyword {
                "title" => Some(Self::Title),
                "x-axis" => Some(Self::XAxis),
                "y-axis" => Range::parse_axis(rest.trim()).map(Self::YAxis),
                "bar" => Self::data(Series::Bar, rest),
                "line" => Self::data(Series::Line, rest),
                _ => None,
            });
        parsed.unwrap_or(Self::Unknown)
    }

    fn data(series: Series, raw: &str) -> Option<Self> {
        let (inner, _) = raw.split_once('[')?.1.rsplit_once(']')?;
        let values: Vec<Value> = inner.split(',').map(Value::parse).collect::<Option<_>>()?;
        let extent = values
            .iter()
            .map(|&value| Range {
                min: value,
                max: value,
            })
            .reduce(Range::including)?;
        Some(Self::Data {
            series,
            count: values.len(),
            extent,
        })
    }
}

struct XySize {
    bars: usize,
    lines: usize,
    widest: usize,
    y: Option<Range>,
    extent: Option<Range>,
}

fn size_of(lines: &[XyLine]) -> XySize {
    let mut size = XySize {
        bars: 0,
        lines: 0,
        widest: 0,
        y: None,
        extent: None,
    };
    for line in lines {
        match *line {
            XyLine::YAxis(range) => size.y = Some(range),
            XyLine::Data {
                series,
                count,
                extent,
            } => {
                match series {
                    Series::Bar => size.bars += 1,
                    Series::Line => size.lines += 1,
                }
                size.widest = size.widest.max(count);
                size.extent = Some(size.extent.map_or(extent, |seen| seen.including(extent)));
            }
            XyLine::Title | XyLine::XAxis | XyLine::Accessibility | XyLine::Unknown => {}
        }
    }
    size
}

pub(super) fn check_header(header: &str) -> Result<(), Refusal> {
    let orientation = strip_inline_comment(header)
        .split_once(char::is_whitespace)
        .map_or("", |(_, rest)| rest.trim());
    if orientation.eq_ignore_ascii_case("horizontal") {
        return Err(Refusal::Unsupported);
    }
    Ok(())
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<XyLine> = body.iter().copied().map(XyLine::parse).collect();
    let size = size_of(&lines);
    let Some(y) = size.y else {
        return Err(Refusal::Unsupported);
    };
    if lines.contains(&XyLine::Unknown)
        || size.bars > 1
        || size.lines > 1
        || size.extent.is_some_and(|extent| !y.contains(extent))
    {
        return Err(Refusal::Unsupported);
    }
    if size.widest > MAX_SERIES_VALUES {
        return Err(Refusal::TooManyPoints);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn xychart_draws_under_both_headers() {
        for header in ["xychart", "xychart-beta"] {
            let source = format!(
                "{header}\n  title \"Sales\"\n  accTitle: x\n  x-axis [jan, feb]\n  y-axis \"Revenue (k)\" 0 --> 10\n  bar [0, 10]\n  line [2.5, 3]"
            );
            assert_draws(&source, DiagramKind::XyChart);
        }
    }

    #[test]
    fn xychart_refuses_charts_the_renderer_draws_wrong() {
        assert_unsupported(&[
            "xychart\n  bar [1, 2]",
            "xychart\n  y-axis 0 --> 9\n  bar [1]\n  bar [2]",
            "xychart\n  y-axis 0 --> 9\n  line [1]\n  line [2]",
            "xychart horizontal %% c\n  y-axis 0 --> 9\n  bar [1]",
            "xychart\n  y-axis 0 --> 9\n  bar [1, 10]",
            "xychart\n  y-axis 2 --> 9\n  line [1, 3]",
        ]);
    }

    #[test]
    fn xychart_refuses_lines_it_cannot_measure() {
        assert_unsupported(&[
            "xychart\n  y-axis 0 --> 9\n  junk",
            "xychart\n  y-axis Revenue\n  bar [1]",
            "xychart\n  y-axis 9 --> 0\n  bar [1]",
            "xychart\n  y-axis 0 --> inf\n  bar [1]",
            "xychart\n  y-axis 0 --> 9\n  bar []",
            "xychart\n  y-axis 0 --> 9\n  bar [1, NaN]",
        ]);
    }

    #[test]
    fn xychart_value_limit_applies_to_each_series() {
        let chart = |n: usize| {
            let values = vec!["1"; n].join(", ");
            format!("xychart\n  y-axis 0 --> 9\n  bar [1]\n  line [{values}]")
        };
        assert_draws(&chart(40), DiagramKind::XyChart);
        assert_eq!(verdict(&chart(41)), Err(Refusal::TooManyPoints));
    }
}
