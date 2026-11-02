use crate::semver::Version;
use flate2::read::GzDecoder;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Installed {
    pub version: Version,
    pub dev: bool,
}

impl Installed {
    pub fn parse(display: &str) -> Result<Self, String> {
        let (base, suffix) = match display.split_once('-') {
            Some((base, suffix)) => (base, Some(suffix)),
            None => (display, None),
        };
        let dev = match suffix {
            None => false,
            Some(suffix) if suffix == "dev" || suffix.starts_with("dev+") => true,
            Some(_) => return Err(format!("unknown ralph version '{display}'")),
        };
        let version = base.parse::<Version>().map_err(|e| e.to_string())?;
        Ok(Self { version, dev })
    }

    fn is_older_than(self, release: Version) -> bool {
        release > self.version || (self.dev && release == self.version)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

pub fn parse_releases(json: &str) -> Result<Vec<Release>, String> {
    let raw: Vec<RawRelease> =
        serde_json::from_str(json).map_err(|e| format!("unexpected GitHub releases reply: {e}"))?;
    let mut releases: Vec<Release> = raw
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let version = release.tag_name.strip_prefix('v')?.parse().ok()?;
            Some(Release {
                version,
                tag: release.tag_name,
                notes: release.body.unwrap_or_default(),
                assets: release.assets,
            })
        })
        .collect();
    releases.sort_by_key(|release| std::cmp::Reverse(release.version));
    Ok(releases)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    UpToDate,
    RefuseDev,
    Install { downgrade: bool },
}

pub fn decide(installed: Installed, latest: Version, force: bool) -> Decision {
    let newer = installed.is_older_than(latest);
    match (installed.dev, force) {
        (true, false) => Decision::RefuseDev,
        (true, true) => Decision::Install { downgrade: !newer },
        (false, _) if newer => Decision::Install { downgrade: false },
        (false, _) => Decision::UpToDate,
    }
}

pub fn skipped_notes(installed: Installed, releases: &[Release]) -> String {
    releases
        .iter()
        .filter(|release| installed.is_older_than(release.version))
        .map(|release| {
            let notes = match release.notes.trim() {
                "" => "(no notes)",
                notes => notes,
            };
            format!("## {}\n\n{notes}", release.tag)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn target_triple(arch: &str, os: &str) -> Option<&'static str> {
    match (arch, os) {
        ("aarch64", "macos") => Some("aarch64-apple-darwin"),
        ("x86_64", "macos") => Some("x86_64-apple-darwin"),
        ("aarch64", "linux") => Some("aarch64-unknown-linux-gnu"),
        ("x86_64", "linux") => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Download<'a> {
    pub archive: &'a Asset,
    pub checksum: &'a Asset,
}

pub fn pick_download<'a>(release: &'a Release, triple: &str) -> Result<Download<'a>, String> {
    let find = |name: &str| {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .ok_or_else(|| format!("{} has no {name}", release.tag))
    };
    let archive_name = format!("ralph-{}-{triple}.tar.gz", release.tag);
    Ok(Download {
        archive: find(&archive_name)?,
        checksum: find(&format!("{archive_name}.sha256"))?,
    })
}

pub fn verify_checksum(file: &str, archive_name: &str, archive: &[u8]) -> Result<(), String> {
    let malformed = || format!("malformed checksum file for {archive_name}");
    let mut fields = file.split_whitespace();
    let (Some(expected), Some(listed), None) = (fields.next(), fields.next(), fields.next()) else {
        return Err(malformed());
    };
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(malformed());
    }
    let listed = listed.trim_start_matches('*');
    if listed != archive_name {
        return Err(format!(
            "checksum file names {listed}, expected {archive_name}"
        ));
    }
    let actual: String = Sha256::digest(archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "checksum mismatch for {archive_name}: expected {expected}, got {actual}"
        ))
    }
}

pub fn extract_binary(archive: &[u8]) -> Result<Vec<u8>, String> {
    let unreadable = |e: std::io::Error| format!("cannot read release archive: {e}");
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    let mut binary = None;
    for entry in tar.entries().map_err(unreadable)? {
        let mut entry = entry.map_err(unreadable)?;
        let is_binary = entry.header().entry_type().is_file()
            && entry.path().map_err(unreadable)? == Path::new("ralph");
        if !is_binary {
            continue;
        }
        if binary.is_some() {
            return Err("release archive has more than one ralph binary".to_string());
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(unreadable)?;
        binary = Some(bytes);
    }
    binary.ok_or_else(|| "release archive has no ralph binary".to_string())
}

#[cfg(test)]
mod tests;
