pub const MAX_HANDLERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Accept,
    Reject,
}

pub(crate) fn admit(active: usize, cap: usize) -> Admission {
    if active < cap {
        Admission::Accept
    } else {
        Admission::Reject
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_cap_accepts() {
        assert_eq!(admit(0, 8), Admission::Accept);
        assert_eq!(admit(7, 8), Admission::Accept);
    }

    #[test]
    fn at_or_over_cap_rejects() {
        assert_eq!(admit(8, 8), Admission::Reject);
        assert_eq!(admit(9, 8), Admission::Reject);
    }

    #[test]
    fn zero_cap_rejects_everything() {
        assert_eq!(admit(0, 0), Admission::Reject);
    }
}
