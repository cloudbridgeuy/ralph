use std::net::Ipv4Addr;

use serde_json::Value;

const PATH_CANDIDATE: &str = "tailscale";
const APP_CANDIDATE: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
const DEFAULT_NOT_RUNNING: &str = "tailscale exited with a failure status";

#[derive(Debug, thiserror::Error)]
pub enum TailnetError {
    #[error("tailscale was not found: tried `tailscale` on PATH and /Applications/Tailscale.app/Contents/MacOS/Tailscale")]
    NotFound,
    #[error("tailscale is not running: {0}")]
    NotRunning(String),
    #[error("tailscale reported no IPv4 address")]
    NoAddress,
    #[error("tailscale reported {0}, which is not in the Tailscale range 100.64.0.0/10")]
    OutsideTailnetRange(Ipv4Addr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnOutcome {
    Missing,
    Ran {
        success: bool,
        stdout: String,
        stderr: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TailnetNames {
    pub short: Option<String>,
    pub fqdn: Option<String>,
}

pub fn tailscale_candidates() -> [&'static str; 2] {
    [PATH_CANDIDATE, APP_CANDIDATE]
}

pub fn classify(outcome: SpawnOutcome) -> Result<String, TailnetError> {
    match outcome {
        SpawnOutcome::Missing => Err(TailnetError::NotFound),
        SpawnOutcome::Ran {
            success: true,
            stdout,
            ..
        } => Ok(stdout),
        SpawnOutcome::Ran { stderr, .. } => Err(TailnetError::NotRunning(
            stderr
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or(DEFAULT_NOT_RUNNING)
                .to_string(),
        )),
    }
}

pub fn pick_outcome(outcomes: &[SpawnOutcome]) -> Result<String, TailnetError> {
    outcomes
        .iter()
        .find(|outcome| **outcome != SpawnOutcome::Missing)
        .cloned()
        .map_or(Err(TailnetError::NotFound), classify)
}

fn usable(ip: &Ipv4Addr) -> bool {
    !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast())
}

pub(crate) fn in_tailnet_range(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    octets[0] == 100 && (octets[1] & 0xC0) == 64
}

pub fn parse_ip_output(stdout: &str) -> Result<Ipv4Addr, TailnetError> {
    let ip = stdout
        .lines()
        .filter_map(|line| line.trim().parse::<Ipv4Addr>().ok())
        .find(usable)
        .ok_or(TailnetError::NoAddress)?;
    if in_tailnet_range(ip) {
        Ok(ip)
    } else {
        Err(TailnetError::OutsideTailnetRange(ip))
    }
}

fn self_field(root: &Value, key: &str) -> Option<String> {
    let value = root.get("Self")?.get(key)?.as_str()?;
    let trimmed = value.trim().trim_end_matches('.').to_lowercase();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn first_label(fqdn: &str) -> Option<String> {
    fqdn.split('.')
        .next()
        .filter(|label| !label.is_empty())
        .map(str::to_string)
}

pub fn parse_status_json(json: &str) -> TailnetNames {
    serde_json::from_str::<Value>(json)
        .map(|root| {
            let fqdn = self_field(&root, "DNSName");
            TailnetNames {
                short: fqdn.as_deref().and_then(first_label),
                fqdn,
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(success: bool, stdout: &str, stderr: &str) -> SpawnOutcome {
        SpawnOutcome::Ran {
            success,
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn ip_prefers_first_v4_over_v6() {
        let ip = parse_ip_output("fd7a:115c::1\n100.64.0.5\n100.64.0.6\n").unwrap();
        assert_eq!(ip, Ipv4Addr::new(100, 64, 0, 5));
    }

    #[test]
    fn ip_v4_then_v6() {
        let ip = parse_ip_output("100.101.102.103\nfd7a:115c:a1e0::1\n").unwrap();
        assert_eq!(ip, Ipv4Addr::new(100, 101, 102, 103));
    }

    #[test]
    fn ip_v6_only_is_no_address() {
        assert!(matches!(
            parse_ip_output("fd7a:115c:a1e0::1\n"),
            Err(TailnetError::NoAddress)
        ));
    }

    #[test]
    fn ip_empty_is_no_address() {
        assert!(matches!(parse_ip_output(""), Err(TailnetError::NoAddress)));
        assert!(matches!(
            parse_ip_output("\n  \n"),
            Err(TailnetError::NoAddress)
        ));
    }

    #[test]
    fn ip_garbage_lines_are_ignored() {
        let ip = parse_ip_output("hello\n\n999.1.1.1\n  100.64.0.9  \n").unwrap();
        assert_eq!(ip, Ipv4Addr::new(100, 64, 0, 9));
    }

    #[test]
    fn ip_rejects_unusable_addresses() {
        for bad in ["127.0.0.1", "0.0.0.0", "224.0.0.1", "255.255.255.255"] {
            assert!(
                matches!(parse_ip_output(bad), Err(TailnetError::NoAddress)),
                "{bad}"
            );
        }
    }

    #[test]
    fn ip_outside_the_tailnet_range_is_refused() {
        for text in ["100.63.255.255", "100.128.0.0", "10.0.0.1", "192.168.1.5"] {
            let ip: Ipv4Addr = text.parse().unwrap();
            match parse_ip_output(text) {
                Err(TailnetError::OutsideTailnetRange(found)) => assert_eq!(found, ip),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn ip_inside_the_tailnet_range_is_accepted() {
        for text in ["100.64.0.0", "100.100.100.100", "100.127.255.255"] {
            assert_eq!(
                parse_ip_output(text).unwrap(),
                text.parse::<Ipv4Addr>().unwrap()
            );
        }
    }

    #[test]
    fn outside_range_error_names_the_range() {
        let message = parse_ip_output("192.168.1.5").unwrap_err().to_string();
        assert!(message.contains("192.168.1.5"), "{message}");
        assert!(message.contains("100.64.0.0/10"), "{message}");
    }

    #[test]
    fn ip_skips_rejected_and_takes_next_v4() {
        let ip = parse_ip_output("127.0.0.1\n100.64.0.2\n").unwrap();
        assert_eq!(ip, Ipv4Addr::new(100, 64, 0, 2));
    }

    #[test]
    fn candidates_probe_path_first() {
        assert_eq!(
            tailscale_candidates(),
            [
                "tailscale",
                "/Applications/Tailscale.app/Contents/MacOS/Tailscale"
            ]
        );
    }

    #[test]
    fn pick_uses_path_when_present() {
        let out = pick_outcome(&[ran(true, "100.64.0.1\n", ""), ran(true, "other", "")]);
        assert_eq!(out.unwrap(), "100.64.0.1\n");
    }

    #[test]
    fn pick_falls_back_to_app_when_path_missing() {
        let out = pick_outcome(&[SpawnOutcome::Missing, ran(true, "100.64.0.7\n", "")]);
        assert_eq!(out.unwrap(), "100.64.0.7\n");
    }

    #[test]
    fn pick_all_missing_is_not_found_naming_both() {
        let err = pick_outcome(&[SpawnOutcome::Missing, SpawnOutcome::Missing]).unwrap_err();
        assert!(matches!(err, TailnetError::NotFound));
        let msg = err.to_string();
        assert!(msg.contains("`tailscale` on PATH"), "{msg}");
        assert!(
            msg.contains("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
            "{msg}"
        );
    }

    #[test]
    fn pick_empty_is_not_found() {
        assert!(matches!(pick_outcome(&[]), Err(TailnetError::NotFound)));
    }

    #[test]
    fn pick_path_failure_does_not_fall_through() {
        let err = pick_outcome(&[
            ran(false, "", "\nTailscale is stopped.\nmore\n"),
            ran(true, "100.64.0.1\n", ""),
        ])
        .unwrap_err();
        match err {
            TailnetError::NotRunning(line) => assert_eq!(line, "Tailscale is stopped."),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn classify_failure_without_stderr_uses_default() {
        match classify(ran(false, "", "  \n")).unwrap_err() {
            TailnetError::NotRunning(line) => assert!(!line.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn classify_missing_is_not_found() {
        assert!(matches!(
            classify(SpawnOutcome::Missing),
            Err(TailnetError::NotFound)
        ));
    }

    #[test]
    fn status_json_extracts_names() {
        let json = r#"{"Version":"1.2","Self":{"HostName":"MacBook-Pro","DNSName":"MacBook-Pro.tail1234.ts.net.","Online":true}}"#;
        let names = parse_status_json(json);
        assert_eq!(names.short.as_deref(), Some("macbook-pro"));
        assert_eq!(names.fqdn.as_deref(), Some("macbook-pro.tail1234.ts.net"));
    }

    #[test]
    fn status_json_short_name_is_the_first_dns_label() {
        let json = r#"{"Self":{"HostName":"Evil-Host","DNSName":"Laptop.tail1234.ts.net."}}"#;
        let names = parse_status_json(json);
        assert_eq!(names.short.as_deref(), Some("laptop"));
        assert_eq!(names.fqdn.as_deref(), Some("laptop.tail1234.ts.net"));
    }

    #[test]
    fn status_json_with_only_host_name_has_no_short_name() {
        let names = parse_status_json(r#"{"Self":{"HostName":"phone"}}"#);
        assert_eq!(names, TailnetNames::default());
        let empty = parse_status_json(r#"{"Self":{"HostName":"phone","DNSName":""}}"#);
        assert_eq!(empty, TailnetNames::default());
    }

    #[test]
    fn status_json_single_label_dns_name_is_both_names() {
        let names = parse_status_json(r#"{"Self":{"DNSName":"Box."}}"#);
        assert_eq!(names.short.as_deref(), Some("box"));
        assert_eq!(names.fqdn.as_deref(), Some("box"));
    }

    #[test]
    fn status_json_missing_fields_is_empty() {
        assert_eq!(parse_status_json(r#"{"Self":{}}"#), TailnetNames::default());
        assert_eq!(parse_status_json("{}"), TailnetNames::default());
    }

    #[test]
    fn status_json_malformed_is_empty() {
        assert_eq!(parse_status_json("not json"), TailnetNames::default());
        assert_eq!(parse_status_json(""), TailnetNames::default());
    }
}
