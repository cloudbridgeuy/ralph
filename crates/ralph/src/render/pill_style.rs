#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PillStyle {
    pub round: bool,
    pub icons: bool,
}

impl PillStyle {
    pub fn resolve(round: bool, icons: bool, emphasis_active: bool) -> Self {
        Self {
            round: round && emphasis_active,
            icons: icons && emphasis_active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_cleared_when_emphasis_is_inactive() {
        assert_eq!(PillStyle::resolve(true, true, false), PillStyle::default());
    }

    #[test]
    fn flags_pass_through_when_emphasis_is_active() {
        assert_eq!(
            PillStyle::resolve(true, false, true),
            PillStyle {
                round: true,
                icons: false
            }
        );
        assert_eq!(
            PillStyle::resolve(false, true, true),
            PillStyle {
                round: false,
                icons: true
            }
        );
    }

    #[test]
    fn default_is_square_and_plain() {
        assert_eq!(PillStyle::default(), PillStyle::resolve(false, false, true));
    }
}
