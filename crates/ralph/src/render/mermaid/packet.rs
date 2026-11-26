use super::lines::Statement;
use super::Refusal;

const MAX_END_BIT: Bit = Bit(1023);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Bit(u32);

impl Bit {
    fn parse(raw: &str) -> Option<Self> {
        raw.trim().parse().ok().map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BitRange {
    start: Bit,
    end: Bit,
}

impl BitRange {
    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if !raw.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        let (start, end) = raw.split_once('-').unwrap_or((raw, raw));
        Some(Self {
            start: Bit::parse(start)?,
            end: Bit::parse(end)?,
        })
    }

    fn highest(self) -> Bit {
        self.start.max(self.end)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacketLine {
    Title,
    Accessibility,
    Field(BitRange),
    Unknown,
}

impl PacketLine {
    fn parse(statement: Statement<'_>) -> Self {
        let code = statement.code;
        if statement.is_accessibility() {
            Self::Accessibility
        } else if code.starts_with("title ") || code.starts_with("title\t") {
            Self::Title
        } else {
            code.split_once(':')
                .and_then(|(range, _)| BitRange::parse(range))
                .map_or(Self::Unknown, Self::Field)
        }
    }
}

struct PacketSize {
    highest: Option<Bit>,
}

fn size_of(lines: &[PacketLine]) -> PacketSize {
    let highest = lines
        .iter()
        .filter_map(|line| match line {
            PacketLine::Field(range) => Some(range.highest()),
            PacketLine::Title | PacketLine::Accessibility | PacketLine::Unknown => None,
        })
        .max();
    PacketSize { highest }
}

pub(super) fn check(body: &[Statement<'_>]) -> Result<(), Refusal> {
    let lines: Vec<PacketLine> = body.iter().copied().map(PacketLine::parse).collect();
    if lines.contains(&PacketLine::Unknown) {
        return Err(Refusal::Unsupported);
    }
    if size_of(&lines).highest > Some(MAX_END_BIT) {
        return Err(Refusal::TooManyBits);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::{assert_draws, assert_unsupported, verdict};
    use super::super::{DiagramKind, Refusal};

    #[test]
    fn packet_draws_under_both_headers() {
        for header in ["packet", "packet-beta"] {
            let source = format!(
                "{header}\n  title T\n  accDescr: x\n  0-15: \"Source\"\n  16 - 30: Dest\n  31: \"F\""
            );
            assert_draws(&source, DiagramKind::Packet);
        }
    }

    #[test]
    fn packet_refuses_lines_the_renderer_drops() {
        assert_unsupported(&[
            "packet\n  0-15: \"a\"\n  +16: \"b\"",
            "packet\n  0-15: \"a\"\n  junk",
            "packet\n  title: T\n  0: \"a\"",
        ]);
    }

    #[test]
    fn packet_bit_limit_covers_both_ends() {
        assert_draws("packet\n  0-1023: \"a\"", DiagramKind::Packet);
        for field in ["0-1024", "1024", "1024-0"] {
            let source = format!("packet\n  {field}: \"a\"");
            assert_eq!(verdict(&source), Err(Refusal::TooManyBits), "{field}");
        }
    }
}
