use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindPolicy {
    pub allowed: Vec<IpAddr>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BindError {
    #[error("refusing to bind {0}: only loopback addresses are allowed")]
    NotAllowed(SocketAddr),
}

impl BindPolicy {
    pub fn loopback() -> BindPolicy {
        BindPolicy {
            allowed: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
        }
    }
}

pub fn check_bind(addr: SocketAddr, policy: &BindPolicy) -> Result<(), BindError> {
    if policy.allowed.contains(&addr.ip()) {
        Ok(())
    } else {
        Err(BindError::NotAllowed(addr))
    }
}

pub fn allowed_hosts(names: &[&str], port: u16) -> Vec<String> {
    names.iter().map(|name| format!("{name}:{port}")).collect()
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
        assert!(allowed_hosts(&[], 4000).is_empty());
    }
}
