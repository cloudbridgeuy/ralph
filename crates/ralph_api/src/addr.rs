use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::tailnet::{in_tailnet_range, TailnetNames};

const LOCALHOST_NAME: &str = "localhost";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindPolicy {
    pub allowed: Vec<IpAddr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BindError {
    #[error("refusing to bind {0}: only loopback addresses are allowed")]
    NotAllowed(SocketAddr),
    #[error("refusing tailnet address {0}: it must be a unicast, specific address")]
    BadTailnetAddress(IpAddr),
    #[error("refusing tailnet address {0}: it is not in the Tailscale range 100.64.0.0/10")]
    OutsideTailnetRange(IpAddr),
}

impl BindPolicy {
    pub fn primary(&self) -> Option<IpAddr> {
        self.allowed.first().copied()
    }

    pub fn loopback() -> BindPolicy {
        BindPolicy {
            allowed: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        }
    }

    pub fn loopback_and(ip: IpAddr) -> Result<BindPolicy, BindError> {
        if ip.is_loopback()
            || ip.is_unspecified()
            || ip.is_multicast()
            || ip == IpAddr::V4(Ipv4Addr::BROADCAST)
        {
            return Err(BindError::BadTailnetAddress(ip));
        }
        match ip {
            IpAddr::V4(v4) if in_tailnet_range(v4) => {}
            _ => return Err(BindError::OutsideTailnetRange(ip)),
        }
        let mut policy = BindPolicy::loopback();
        policy.allowed.push(ip);
        Ok(policy)
    }
}

pub fn tailnet_hosts(ip: Ipv4Addr, names: &TailnetNames, port: u16) -> Vec<String> {
    let mut hosts = allowed_hosts(
        &[
            Ipv4Addr::LOCALHOST.to_string(),
            LOCALHOST_NAME.to_string(),
            ip.to_string(),
        ],
        port,
    );
    let named = [names.short.as_deref(), names.fqdn.as_deref()]
        .into_iter()
        .flatten()
        .filter(|name| !name.is_empty());
    for host in allowed_hosts(&named.collect::<Vec<_>>(), port) {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

pub fn bind_hosts(ip: IpAddr) -> Vec<String> {
    vec![ip.to_string(), LOCALHOST_NAME.to_string()]
}

fn host_with_port(name: &str, port: u16) -> String {
    match name.parse::<Ipv6Addr>() {
        Ok(_) => format!("[{name}]:{port}"),
        Err(_) => format!("{name}:{port}"),
    }
}

pub(crate) fn check_bind(addr: SocketAddr, policy: &BindPolicy) -> Result<(), BindError> {
    if policy.allowed.contains(&addr.ip()) {
        Ok(())
    } else {
        Err(BindError::NotAllowed(addr))
    }
}

pub fn allowed_hosts<S: AsRef<str>>(names: &[S], port: u16) -> Vec<String> {
    names
        .iter()
        .map(|name| host_with_port(name.as_ref(), port))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn loopback_v4_ok() {
        assert_eq!(
            check_bind(addr("127.0.0.1:4000"), &BindPolicy::loopback()),
            Ok(())
        );
    }

    #[test]
    fn loopback_v6_ok() {
        assert_eq!(
            check_bind(addr("[::1]:4000"), &BindPolicy::loopback()),
            Ok(())
        );
    }

    #[test]
    fn any_addr_refused() {
        assert!(check_bind(addr("0.0.0.0:4000"), &BindPolicy::loopback()).is_err());
        assert!(check_bind(addr("[::]:4000"), &BindPolicy::loopback()).is_err());
    }

    #[test]
    fn lan_addr_refused() {
        assert!(check_bind(addr("192.168.1.20:4000"), &BindPolicy::loopback()).is_err());
        assert!(check_bind(addr("10.0.0.5:0"), &BindPolicy::loopback()).is_err());
    }

    #[test]
    fn custom_policy_allows_added_ip() {
        let lan = addr("192.168.1.20:4000");
        let mut policy = BindPolicy::loopback();
        assert!(check_bind(lan, &policy).is_err());
        policy.allowed.push(lan.ip());
        assert_eq!(check_bind(lan, &policy), Ok(()));
        let empty = BindPolicy { allowed: vec![] };
        assert!(check_bind(addr("127.0.0.1:4000"), &empty).is_err());
    }

    #[test]
    fn check_bind_error_names_the_address() {
        let error = check_bind(addr("0.0.0.0:4000"), &BindPolicy::loopback()).unwrap_err();
        assert!(error.to_string().contains("0.0.0.0:4000"));
    }

    #[test]
    fn allowed_hosts_builds_host_port_pairs() {
        assert_eq!(
            allowed_hosts(&["127.0.0.1", "localhost"], 4000),
            vec!["127.0.0.1:4000".to_string(), "localhost:4000".to_string()]
        );
        assert!(allowed_hosts::<&str>(&[], 4000).is_empty());
    }

    #[test]
    fn primary_is_the_first_allowed_ip() {
        assert_eq!(
            BindPolicy::loopback().primary(),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
        assert_eq!(BindPolicy { allowed: vec![] }.primary(), None);
    }

    #[test]
    fn bind_hosts_names_the_ip_and_localhost() {
        assert_eq!(
            bind_hosts(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            vec!["127.0.0.1".to_string(), "localhost".to_string()]
        );
        assert_eq!(
            bind_hosts(IpAddr::V6(Ipv6Addr::LOCALHOST)),
            vec!["::1".to_string(), "localhost".to_string()]
        );
    }

    #[test]
    fn allowed_hosts_brackets_ipv6() {
        assert_eq!(
            allowed_hosts(&["::1", "localhost"], 4000),
            vec!["[::1]:4000".to_string(), "localhost:4000".to_string()]
        );
    }

    fn tailnet() -> IpAddr {
        "100.101.102.103".parse().unwrap()
    }

    #[test]
    fn loopback_and_allows_loopback_and_exactly_the_tailnet_ip() {
        let policy = BindPolicy::loopback_and(tailnet()).unwrap();
        assert_eq!(check_bind(addr("127.0.0.1:4000"), &policy), Ok(()));
        assert_eq!(check_bind(addr("[::1]:4000"), &policy), Ok(()));
        assert_eq!(check_bind(addr("100.101.102.103:4000"), &policy), Ok(()));
        assert_eq!(check_bind(addr("100.101.102.103:0"), &policy), Ok(()));
    }

    #[test]
    fn loopback_and_refuses_every_other_address() {
        let policy = BindPolicy::loopback_and(tailnet()).unwrap();
        assert!(check_bind(addr("100.101.102.104:4000"), &policy).is_err());
        assert!(check_bind(addr("0.0.0.0:4000"), &policy).is_err());
        assert!(check_bind(addr("[::]:4000"), &policy).is_err());
        assert!(check_bind(addr("192.168.1.20:4000"), &policy).is_err());
    }

    #[test]
    fn default_policy_still_refuses_the_tailnet_ip_and_any_addr() {
        let policy = BindPolicy::loopback();
        assert!(check_bind(addr("100.101.102.103:4000"), &policy).is_err());
        assert!(check_bind(addr("0.0.0.0:4000"), &policy).is_err());
        assert_eq!(policy.allowed.len(), 2);
    }

    #[test]
    fn loopback_and_refuses_unusable_addresses() {
        for text in [
            "127.0.0.1",
            "127.0.0.2",
            "::1",
            "0.0.0.0",
            "::",
            "224.0.0.1",
            "255.255.255.255",
            "ff02::1",
        ] {
            let ip: IpAddr = text.parse().unwrap();
            assert_eq!(
                BindPolicy::loopback_and(ip),
                Err(BindError::BadTailnetAddress(ip)),
                "{text}"
            );
        }
    }

    #[test]
    fn loopback_and_refuses_addresses_outside_the_tailnet_range() {
        for text in ["100.63.255.255", "100.128.0.0", "10.0.0.1", "192.168.1.5"] {
            let ip: IpAddr = text.parse().unwrap();
            assert_eq!(
                BindPolicy::loopback_and(ip),
                Err(BindError::OutsideTailnetRange(ip)),
                "{text}"
            );
        }
    }

    #[test]
    fn loopback_and_accepts_the_tailnet_range_edges() {
        for text in ["100.64.0.0", "100.100.100.100", "100.127.255.255"] {
            let ip: IpAddr = text.parse().unwrap();
            assert!(BindPolicy::loopback_and(ip).is_ok(), "{text}");
        }
    }

    #[test]
    fn loopback_and_keeps_loopback_first() {
        let policy = BindPolicy::loopback_and(tailnet()).unwrap();
        assert_eq!(policy.primary(), Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert_eq!(policy.allowed.len(), 3);
    }

    fn names(short: Option<&str>, fqdn: Option<&str>) -> TailnetNames {
        TailnetNames {
            short: short.map(str::to_string),
            fqdn: fqdn.map(str::to_string),
        }
    }

    #[test]
    fn tailnet_hosts_lists_loopback_ip_and_both_names() {
        assert_eq!(
            tailnet_hosts(
                Ipv4Addr::new(100, 101, 102, 103),
                &names(Some("phone"), Some("phone.tail1234.ts.net")),
                8080
            ),
            vec![
                "127.0.0.1:8080".to_string(),
                "localhost:8080".to_string(),
                "100.101.102.103:8080".to_string(),
                "phone:8080".to_string(),
                "phone.tail1234.ts.net:8080".to_string(),
            ]
        );
    }

    #[test]
    fn tailnet_hosts_skips_missing_empty_and_duplicate_names() {
        let ip = Ipv4Addr::new(100, 101, 102, 103);
        assert_eq!(
            tailnet_hosts(ip, &names(None, None), 8080),
            vec![
                "127.0.0.1:8080".to_string(),
                "localhost:8080".to_string(),
                "100.101.102.103:8080".to_string(),
            ]
        );
        assert_eq!(tailnet_hosts(ip, &names(Some(""), Some("")), 8080).len(), 3);
        assert_eq!(
            tailnet_hosts(ip, &names(Some("phone"), Some("phone")), 8080).len(),
            4
        );
    }
}
