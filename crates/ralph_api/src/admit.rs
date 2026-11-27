use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};

pub const MAX_HANDLERS: usize = 8;
pub const MAX_REJECTED_PER_PEER: usize = 2;

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

pub(crate) fn counts_toward_peer_cap(status: u16) -> bool {
    matches!(status, 401 | 403 | 413)
}

pub(crate) fn counted_peer(ip: Option<IpAddr>) -> Option<IpAddr> {
    ip.map(|ip| ip.to_canonical())
        .filter(|ip| !ip.is_loopback())
}

#[derive(Debug, Default)]
pub(crate) struct PeerCounts(Mutex<HashMap<IpAddr, usize>>);

pub(crate) struct PeerGuard {
    counts: Arc<PeerCounts>,
    ip: IpAddr,
}

impl PeerCounts {
    pub(crate) fn try_enter(counts: &Arc<PeerCounts>, ip: IpAddr, cap: usize) -> Option<PeerGuard> {
        let mut map = counts.0.lock().unwrap_or_else(PoisonError::into_inner);
        let current = map.get(&ip).copied().unwrap_or(0);
        match admit(current, cap) {
            Admission::Reject => None,
            Admission::Accept => {
                map.insert(ip, current + 1);
                Some(PeerGuard {
                    counts: Arc::clone(counts),
                    ip,
                })
            }
        }
    }

    #[cfg(test)]
    fn count(&self, ip: IpAddr) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&ip)
            .copied()
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

impl Drop for PeerGuard {
    fn drop(&mut self) {
        let mut map = self.counts.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = map.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn peer(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(100, 64, 0, last))
    }

    #[test]
    fn rejected_statuses_count() {
        for status in [401, 403, 413] {
            assert!(counts_toward_peer_cap(status), "{status}");
        }
    }

    #[test]
    fn other_statuses_never_count() {
        for status in [200, 204, 301, 304, 400, 404, 405, 408, 500, 503] {
            assert!(!counts_toward_peer_cap(status), "{status}");
        }
    }

    #[test]
    fn loopback_and_absent_peers_are_exempt() {
        assert_eq!(counted_peer(None), None);
        assert_eq!(counted_peer(Some(IpAddr::V4(Ipv4Addr::LOCALHOST))), None);
        assert_eq!(
            counted_peer(Some(IpAddr::V4(Ipv4Addr::new(127, 9, 9, 9)))),
            None
        );
        assert_eq!(counted_peer(Some(IpAddr::V6(Ipv6Addr::LOCALHOST))), None);
        let mapped = IpAddr::V6(Ipv4Addr::new(127, 0, 0, 1).to_ipv6_mapped());
        assert_eq!(counted_peer(Some(mapped)), None);
    }

    #[test]
    fn other_peers_are_counted() {
        assert_eq!(counted_peer(Some(peer(7))), Some(peer(7)));
        let mapped = IpAddr::V6(Ipv4Addr::new(100, 64, 0, 7).to_ipv6_mapped());
        assert_eq!(counted_peer(Some(mapped)), Some(peer(7)));
    }

    #[test]
    fn try_enter_increments_below_the_cap() {
        let counts = Arc::new(PeerCounts::default());
        let ip = peer(1);
        let first = PeerCounts::try_enter(&counts, ip, MAX_REJECTED_PER_PEER);
        assert!(first.is_some());
        assert_eq!(counts.count(ip), 1);
        let second = PeerCounts::try_enter(&counts, ip, MAX_REJECTED_PER_PEER);
        assert!(second.is_some());
        assert_eq!(counts.count(ip), 2);
    }

    #[test]
    fn try_enter_at_the_cap_returns_none_and_leaves_the_count() {
        let counts = Arc::new(PeerCounts::default());
        let ip = peer(1);
        let held: Vec<_> = (0..MAX_REJECTED_PER_PEER)
            .map(|_| PeerCounts::try_enter(&counts, ip, MAX_REJECTED_PER_PEER))
            .collect();
        assert!(held.iter().all(Option::is_some));
        assert!(PeerCounts::try_enter(&counts, ip, MAX_REJECTED_PER_PEER).is_none());
        assert_eq!(counts.count(ip), MAX_REJECTED_PER_PEER);
    }

    #[test]
    fn try_enter_with_a_zero_cap_tracks_nothing() {
        let counts = Arc::new(PeerCounts::default());
        assert!(PeerCounts::try_enter(&counts, peer(1), 0).is_none());
        assert_eq!(counts.tracked(), 0);
    }

    #[test]
    fn dropping_guards_restores_the_count_and_removes_the_entry() {
        let counts = Arc::new(PeerCounts::default());
        let ip = peer(1);
        let first = PeerCounts::try_enter(&counts, ip, MAX_REJECTED_PER_PEER);
        let second = PeerCounts::try_enter(&counts, ip, MAX_REJECTED_PER_PEER);
        drop(first);
        assert_eq!(counts.count(ip), 1);
        drop(second);
        assert_eq!(counts.count(ip), 0);
        assert_eq!(counts.tracked(), 0);
    }

    #[test]
    fn peers_are_counted_independently() {
        let counts = Arc::new(PeerCounts::default());
        let _a1 = PeerCounts::try_enter(&counts, peer(1), MAX_REJECTED_PER_PEER);
        let _a2 = PeerCounts::try_enter(&counts, peer(1), MAX_REJECTED_PER_PEER);
        let b = PeerCounts::try_enter(&counts, peer(2), MAX_REJECTED_PER_PEER);
        assert!(b.is_some());
        assert_eq!(counts.tracked(), 2);
        drop(b);
        assert_eq!(counts.tracked(), 1);
    }

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
