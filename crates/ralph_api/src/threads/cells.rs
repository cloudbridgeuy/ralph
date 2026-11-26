#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Plain,
    Ok,
    Warn,
    Bad,
}

impl Tone {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Tone::Plain => "plain",
            Tone::Ok => "ok",
            Tone::Warn => "warn",
            Tone::Bad => "bad",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) ch: char,
    pub(crate) tone: Tone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    Head,
    Open,
    Parked,
    Merged,
    Dropped,
}

const fn cell(ch: char, tone: Tone) -> Cell {
    Cell { ch, tone }
}

pub(crate) const fn lane_cells(lane: Lane) -> [Cell; 3] {
    match lane {
        Lane::Head => [
            cell('●', Tone::Plain),
            cell(' ', Tone::Plain),
            cell(' ', Tone::Plain),
        ],
        Lane::Open => [
            cell('├', Tone::Plain),
            cell('─', Tone::Plain),
            cell('○', Tone::Plain),
        ],
        Lane::Parked => [
            cell('├', Tone::Plain),
            cell('─', Tone::Warn),
            cell('○', Tone::Warn),
        ],
        Lane::Merged => [
            cell('├', Tone::Plain),
            cell('◄', Tone::Ok),
            cell('●', Tone::Ok),
        ],
        Lane::Dropped => [
            cell('╰', Tone::Plain),
            cell('─', Tone::Bad),
            cell('✕', Tone::Bad),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyphs(lane: Lane) -> String {
        lane_cells(lane).iter().map(|cell| cell.ch).collect()
    }

    fn tones(lane: Lane) -> Vec<&'static str> {
        lane_cells(lane)
            .iter()
            .map(|cell| cell.tone.name())
            .collect()
    }

    #[test]
    fn each_lane_has_the_designed_glyphs() {
        assert_eq!(glyphs(Lane::Head), "●  ");
        assert_eq!(glyphs(Lane::Open), "├─○");
        assert_eq!(glyphs(Lane::Parked), "├─○");
        assert_eq!(glyphs(Lane::Merged), "├◄●");
        assert_eq!(glyphs(Lane::Dropped), "╰─✕");
    }

    #[test]
    fn the_trunk_cell_is_always_plain() {
        for lane in [
            Lane::Head,
            Lane::Open,
            Lane::Parked,
            Lane::Merged,
            Lane::Dropped,
        ] {
            assert_eq!(lane_cells(lane)[0].tone, Tone::Plain);
        }
    }

    #[test]
    fn lane_and_tip_carry_the_state_tone() {
        assert_eq!(tones(Lane::Open), ["plain", "plain", "plain"]);
        assert_eq!(tones(Lane::Parked), ["plain", "warn", "warn"]);
        assert_eq!(tones(Lane::Merged), ["plain", "ok", "ok"]);
        assert_eq!(tones(Lane::Dropped), ["plain", "bad", "bad"]);
    }
}
