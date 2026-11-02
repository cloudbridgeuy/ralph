use crate::cli::UpgradeArgs;
use ralph_core::upgrade::{
    decide, extract_binary, parse_releases, pick_download, skipped_notes, target_triple,
    verify_checksum, Decision, Installed, Release,
};
use std::env::consts::{ARCH, OS};
use std::error::Error;
use std::fs::{self, Permissions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;
use ureq::Agent;

const TARGET_ENV: &str = if cfg!(target_env = "musl") {
    "musl"
} else {
    ""
};
const RELEASES_URL: &str = "https://api.github.com/repos/cloudbridgeuy/ralph/releases?per_page=100";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const ARCHIVE_LIMIT: u64 = 128 * 1024 * 1024;
const RESTART_NOTE: &str = "Running ralph chats and workers keep the old build until they restart.";

pub fn run(args: UpgradeArgs) -> Result<(), Box<dyn Error>> {
    let current = crate::version::current();
    let installed = Installed::parse(current)?;
    let agent: Agent = Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .user_agent(format!("ralph/{current}"))
        .build()
        .into();
    let releases = parse_releases(&get_text(&agent, RELEASES_URL)?)?;
    let Some(latest) = releases.first() else {
        println!("No published ralph release found.");
        return Ok(());
    };
    let decision = decide(installed, latest.version, args.force);
    let notes = skipped_notes(installed, &releases);
    if args.check {
        println!(
            "ralph {current} installed; latest release is {}.",
            latest.tag
        );
        println!("{}", check_summary(decision));
        print_notes(&notes, decision);
        return Ok(());
    }
    match decision {
        Decision::UpToDate => {
            println!(
                "ralph {current} is up to date (latest release {}).",
                latest.tag
            );
            Ok(())
        }
        Decision::RefuseDev => Err(format!(
            "ralph {current} is a dev build; refusing to replace it with release {}. \
             Run `ralph upgrade --force` to replace it anyway.",
            latest.tag
        )
        .into()),
        Decision::Install { downgrade } => {
            let path = install(&agent, latest)?;
            print_notes(&notes, decision);
            let verb = if downgrade {
                "Replaced dev build"
            } else {
                "Upgraded"
            };
            println!(
                "{verb} ralph {current} with {} at {}.",
                latest.tag,
                path.display()
            );
            println!("{RESTART_NOTE}");
            Ok(())
        }
    }
}

fn check_summary(decision: Decision) -> &'static str {
    match decision {
        Decision::UpToDate => "Up to date.",
        Decision::RefuseDev => "Dev build: `ralph upgrade` refuses without --force.",
        Decision::Install { downgrade: true } => {
            "`ralph upgrade` would replace this dev build with an older release."
        }
        Decision::Install { downgrade: false } => "Upgrade available: run `ralph upgrade`.",
    }
}

fn print_notes(notes: &str, decision: Decision) {
    if matches!(decision, Decision::Install { .. }) && !notes.is_empty() {
        println!("\n{notes}\n");
    }
}

fn install(agent: &Agent, release: &Release) -> Result<PathBuf, Box<dyn Error>> {
    let triple = target_triple(ARCH, OS, TARGET_ENV)?;
    let download = pick_download(release, triple)?;
    let exe = std::env::current_exe()?.canonicalize()?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", exe.display()))?;
    let mut staged = tempfile::Builder::new()
        .prefix(".ralph-upgrade-")
        .tempfile_in(dir)
        .map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
    let archive = get_bytes(agent, &download.archive.browser_download_url)?;
    let checksum = get_text(agent, &download.checksum.browser_download_url)?;
    verify_checksum(&checksum, &download.archive.name, &archive)?;
    staged.write_all(&extract_binary(&archive)?)?;
    staged.as_file().sync_all()?;
    fs::set_permissions(staged.path(), Permissions::from_mode(0o755))?;
    staged
        .persist(&exe)
        .map_err(|e| format!("cannot replace {}: {}", exe.display(), e.error))?;
    Ok(exe)
}

fn unreachable_github(url: &str, error: ureq::Error) -> String {
    format!("cannot reach GitHub ({url}): {error}")
}

fn get_text(agent: &Agent, url: &str) -> Result<String, String> {
    agent
        .get(url)
        .call()
        .and_then(|mut response| response.body_mut().read_to_string())
        .map_err(|e| unreachable_github(url, e))
}

fn get_bytes(agent: &Agent, url: &str) -> Result<Vec<u8>, String> {
    agent
        .get(url)
        .call()
        .and_then(|mut response| {
            response
                .body_mut()
                .with_config()
                .limit(ARCHIVE_LIMIT)
                .read_to_vec()
        })
        .map_err(|e| unreachable_github(url, e))
}
