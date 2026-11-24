use ralph_api::{
    parse_ip_output, parse_status_json, pick_outcome, tailscale_candidates, SpawnOutcome,
    TailnetError, TailnetNames,
};
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::process::Command;

pub struct Tailnet {
    pub ip: Ipv4Addr,
    pub names: TailnetNames,
}

fn run_tailscale(bin: &str, args: &[&str]) -> SpawnOutcome {
    match Command::new(bin).args(args).output() {
        Ok(output) => SpawnOutcome::Ran {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => SpawnOutcome::Missing,
        Err(error) => SpawnOutcome::Ran {
            success: false,
            stdout: String::new(),
            stderr: error.to_string(),
        },
    }
}

pub fn discover() -> Result<Tailnet, TailnetError> {
    let candidates = tailscale_candidates();
    let mut outcomes = Vec::new();
    for bin in candidates {
        let outcome = run_tailscale(bin, &["ip"]);
        let missing = outcome == SpawnOutcome::Missing;
        outcomes.push(outcome);
        if !missing {
            break;
        }
    }
    let ip = parse_ip_output(&pick_outcome(&outcomes)?)?;
    let names = candidates
        .get(outcomes.len() - 1)
        .map(|bin| status_names(bin))
        .unwrap_or_default();
    Ok(Tailnet { ip, names })
}

fn status_names(bin: &str) -> TailnetNames {
    match run_tailscale(bin, &["status", "--json"]) {
        SpawnOutcome::Ran {
            success: true,
            stdout,
            ..
        } => {
            let names = parse_status_json(&stdout);
            if names == TailnetNames::default() {
                eprintln!("warning: tailscale status gave no host names; using the IP only");
            }
            names
        }
        _ => {
            eprintln!("warning: tailscale status failed; using the IP only");
            TailnetNames::default()
        }
    }
}
