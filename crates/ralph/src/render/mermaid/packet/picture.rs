use super::super::Refusal;
use super::{Bit, BitRange};

const MAX_PICTURE_END_BIT: Bit = Bit(4095);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PictureField {
    Range(BitRange),
    Width(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PacketLine {
    Field(PictureField),
    Other,
    Invalid(Refusal),
}

fn is_number(raw: &str) -> bool {
    !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit())
}

impl PictureField {
    fn parse(line: &str) -> PacketLine {
        let line = line.trim();
        let line = line
            .strip_prefix("packet-beta")
            .or_else(|| line.strip_prefix("packet"))
            .unwrap_or(line);
        let Some((bits, _)) = line.split_once(':') else {
            return PacketLine::Other;
        };
        let bits = bits.trim();
        let field = if let Some(width) = bits.strip_prefix('+').map(str::trim) {
            if !is_number(width) {
                return PacketLine::Other;
            }
            width.parse().ok().map(Self::Width)
        } else {
            let (start, end) = bits.split_once('-').unwrap_or((bits, bits));
            if !is_number(start.trim()) || !is_number(end.trim()) {
                return PacketLine::Other;
            }
            BitRange::parse(bits).map(Self::Range)
        };
        field.map_or(PacketLine::Invalid(Refusal::TooManyBits), PacketLine::Field)
    }

    fn highest(self, next: Bit) -> Bit {
        match self {
            Self::Range(range) => range.highest(),
            Self::Width(width) => Bit(next.0.saturating_add(width).saturating_sub(1)),
        }
    }
}

pub(in super::super) fn check(source: &str) -> Result<(), Refusal> {
    let mut next = Bit(0);
    for line in source.lines() {
        let field = match PictureField::parse(line) {
            PacketLine::Field(field) => field,
            PacketLine::Other => continue,
            PacketLine::Invalid(refusal) => return Err(refusal),
        };
        let highest = field.highest(next);
        if highest > MAX_PICTURE_END_BIT {
            return Err(Refusal::TooManyBits);
        }
        next = Bit(highest.0 + 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::{picture_precheck, Refusal};
    use ralph_server::config::MermaidSection;

    fn verdict(source: &str) -> Result<(), Refusal> {
        picture_precheck(source, &MermaidSection::default())
    }

    #[test]
    fn huge_packets_are_refused_in_every_field_form() {
        for source in [
            "packet\n  0-30000000: \"x\"",
            "packet-beta\n  30000000: \"x\"",
            "packet\n  30000000-0: \"x\"",
            "packet\n  +30000000: \"x\"",
            "packet\n  0-4095: \"a\"\n  +1: \"b\"",
            "packet\n  +4000: \"a\"\n  +97: \"b\"",
            "packet\n  0-99999999999: \"x\"",
            "packet\n  +99999999999: \"x\"",
            "packet 0-30000000: \"x\"",
            "packet-beta+30000000: \"x\"",
            "packet\r  0-30000000: \"x\"",
            "packet\n  %%{init: {}}%% 0-30000000: \"x\"",
            "%% note\npacket\n  0 - 30000000 : \"x\" %% c",
            "%%{init: {\"theme\": \"dark\"}}%%\npacket\n  0-30000000: \"x\"",
            "---\nconfig:\n  packet:\n    bitsPerRow: 1\n---\npacket\n  0-30000000: \"x\"",
        ] {
            assert_eq!(verdict(source), Err(Refusal::TooManyBits), "{source:?}");
        }
    }

    #[test]
    fn the_compact_forms_are_ones_the_renderer_reads() {
        for source in [
            "packet 0-7: \"x\"",
            "packet-beta+8: \"x\"",
            "packet\r  0-7: \"x\"",
            "packet\n  %%{init: {}}%% 0-7: \"x\"",
        ] {
            let parsed = bezel::parse(source, &bezel::Options::default());
            assert!(parsed.is_ok(), "{source:?}");
        }
    }

    #[test]
    fn packets_up_to_the_bit_limit_are_admitted() {
        for source in [
            "packet\n  0-4095: \"a\"",
            "packet\n  +4000: \"a\"\n  +96: \"b\"",
            "packet\n  title 30000000 bits\n  0-15: \"a: 99999\"\n  junk",
        ] {
            assert_eq!(verdict(source), Ok(()), "{source:?}");
        }
    }

    #[test]
    fn other_diagrams_are_left_to_the_renderer() {
        let chain: String = (0..500).map(|i| format!("  N{i}-->N{}\n", i + 1)).collect();
        for source in [
            "block\n  a",
            "architecture-beta\n  service a(server)[A]",
            "pie\n  title Pets\n  \"Dogs\" : 3",
            "sequenceDiagram\n  30000000->>B: hi",
            &format!("flowchart TD\n{chain}"),
        ] {
            assert_eq!(verdict(source), Ok(()), "{source:?}");
        }
    }

    #[test]
    fn the_input_byte_limit_holds_for_pictures() {
        let limits = MermaidSection {
            max_input_bytes: 8,
            ..MermaidSection::default()
        };
        assert_eq!(
            picture_precheck("graph TD\n A-->B", &limits),
            Err(Refusal::TooLarge)
        );
    }
}
