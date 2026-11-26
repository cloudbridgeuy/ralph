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

const fn trunk_cell(last: bool) -> Cell {
    cell(if last { '╰' } else { '├' }, Tone::Plain)
}

pub(crate) const fn lane_cells(lane: Lane, last: bool) -> [Cell; 3] {
    let trunk = trunk_cell(last);
    match lane {
        Lane::Head => [
            cell('●', Tone::Plain),
            cell(' ', Tone::Plain),
            cell(' ', Tone::Plain),
        ],
        Lane::Open => [trunk, cell('─', Tone::Plain), cell('○', Tone::Plain)],
        Lane::Parked => [trunk, cell('─', Tone::Warn), cell('○', Tone::Warn)],
        Lane::Merged => [trunk, cell('◄', Tone::Ok), cell('●', Tone::Ok)],
        Lane::Dropped => [trunk, cell('─', Tone::Bad), cell('✕', Tone::Bad)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Lane; 5] = [
        Lane::Head,
        Lane::Open,
        Lane::Parked,
        Lane::Merged,
        Lane::Dropped,
    ];

    fn glyphs(lane: Lane, last: bool) -> String {
        lane_cells(lane, last).iter().map(|cell| cell.ch).collect()
    }

    fn tones(lane: Lane, last: bool) -> Vec<&'static str> {
        lane_cells(lane, last)
            .iter()
            .map(|cell| cell.tone.name())
            .collect()
    }

    #[test]
    fn each_lane_has_the_designed_glyphs_when_not_last() {
        assert_eq!(glyphs(Lane::Head, false), "●  ");
        assert_eq!(glyphs(Lane::Open, false), "├─○");
        assert_eq!(glyphs(Lane::Parked, false), "├─○");
        assert_eq!(glyphs(Lane::Merged, false), "├◄●");
        assert_eq!(glyphs(Lane::Dropped, false), "├─✕");
    }

    #[test]
    fn the_last_row_ends_the_trunk_with_a_curve_except_the_head() {
        assert_eq!(glyphs(Lane::Head, true), "●  ");
        assert_eq!(glyphs(Lane::Open, true), "╰─○");
        assert_eq!(glyphs(Lane::Parked, true), "╰─○");
        assert_eq!(glyphs(Lane::Merged, true), "╰◄●");
        assert_eq!(glyphs(Lane::Dropped, true), "╰─✕");
    }

    #[test]
    fn the_trunk_cell_is_always_plain() {
        for lane in ALL {
            for last in [false, true] {
                assert_eq!(lane_cells(lane, last)[0].tone, Tone::Plain);
            }
        }
    }

    #[test]
    fn lane_and_tip_carry_the_state_tone_whether_or_not_last() {
        for last in [false, true] {
            assert_eq!(tones(Lane::Open, last), ["plain", "plain", "plain"]);
            assert_eq!(tones(Lane::Parked, last), ["plain", "warn", "warn"]);
            assert_eq!(tones(Lane::Merged, last), ["plain", "ok", "ok"]);
            assert_eq!(tones(Lane::Dropped, last), ["plain", "bad", "bad"]);
        }
    }
}
