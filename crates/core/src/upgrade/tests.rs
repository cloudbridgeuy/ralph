use super::*;
use flate2::write::GzEncoder;
use flate2::Compression;

fn v(text: &str) -> Version {
    text.parse().unwrap()
}

fn installed(text: &str) -> Installed {
    Installed::parse(text).unwrap()
}

fn asset(name: &str) -> Asset {
    Asset {
        name: name.to_string(),
        browser_download_url: format!("https://example.test/{name}"),
    }
}

fn release(tag: &str, notes: &str, assets: &[&str]) -> Release {
    Release {
        version: v(tag.trim_start_matches('v')),
        tag: tag.to_string(),
        notes: notes.to_string(),
        assets: assets.iter().map(|name| asset(name)).collect(),
    }
}

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    for (path, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append_data(&mut header, path, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn parses_release_and_dev_versions() {
    assert_eq!(
        installed("0.2.0"),
        Installed {
            version: v("0.2.0"),
            dev: false
        }
    );
    for dev in ["0.2.1-dev", "0.2.1-dev+01c55e8", "0.2.1-dev+01c55e8.dirty"] {
        assert_eq!(
            installed(dev),
            Installed {
                version: v("0.2.1"),
                dev: true
            },
            "{dev}"
        );
    }
}

#[test]
fn rejects_unknown_version_shapes() {
    for bad in ["", "v0.2.0", "0.2", "0.2.1-rc1", "0.2.1+sha"] {
        assert!(Installed::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn keeps_published_releases_newest_first() {
    let json = r#"[
        {"tag_name":"v0.2.0","body":"two","draft":false,"prerelease":false,"assets":[{"name":"a","browser_download_url":"https://x/a","size":1}]},
        {"tag_name":"v0.10.0","body":null,"draft":false,"prerelease":false,"assets":[]},
        {"tag_name":"v0.11.0","body":"draft","draft":true,"prerelease":false,"assets":[]},
        {"tag_name":"v0.12.0","body":"pre","draft":false,"prerelease":true,"assets":[]},
        {"tag_name":"nightly","body":"","draft":false,"prerelease":false,"assets":[]}
    ]"#;
    let releases = parse_releases(json).unwrap();
    assert_eq!(
        releases,
        vec![
            release("v0.10.0", "", &[]),
            Release {
                version: v("0.2.0"),
                tag: "v0.2.0".to_string(),
                notes: "two".to_string(),
                assets: vec![Asset {
                    name: "a".to_string(),
                    browser_download_url: "https://x/a".to_string(),
                }],
            },
        ]
    );
}

#[test]
fn rejects_a_body_that_is_not_a_release_list() {
    assert!(parse_releases(r#"{"message":"API rate limit exceeded"}"#).is_err());
}

#[test]
fn decides_for_release_builds() {
    let current = installed("0.2.0");
    assert_eq!(decide(current, v("0.2.0"), false), Decision::UpToDate);
    assert_eq!(decide(current, v("0.1.9"), true), Decision::UpToDate);
    assert_eq!(
        decide(current, v("0.10.0"), false),
        Decision::Install { downgrade: false }
    );
}

#[test]
fn refuses_dev_builds_without_force() {
    let current = installed("0.2.1-dev+abc");
    assert_eq!(decide(current, v("0.3.0"), false), Decision::RefuseDev);
    assert_eq!(
        decide(current, v("0.2.0"), true),
        Decision::Install { downgrade: true }
    );
    assert_eq!(
        decide(current, v("0.2.1"), true),
        Decision::Install { downgrade: false }
    );
}

#[test]
fn notes_cover_every_skipped_release_newest_first() {
    let releases = [
        release("v0.4.0", "four\n", &[]),
        release("v0.3.0", "", &[]),
        release("v0.2.0", "two", &[]),
    ];
    assert_eq!(
        skipped_notes(installed("0.2.0"), &releases),
        "## v0.4.0\n\nfour\n\n## v0.3.0\n\n(no notes)"
    );
    assert_eq!(
        skipped_notes(installed("0.2.0-dev+abc"), &releases),
        "## v0.4.0\n\nfour\n\n## v0.3.0\n\n(no notes)\n\n## v0.2.0\n\ntwo"
    );
    assert_eq!(skipped_notes(installed("0.4.0"), &releases), "");
}

#[test]
fn maps_supported_platforms_to_targets() {
    for env in ["", "gnu"] {
        assert_eq!(
            target_triple("aarch64", "macos", env),
            Ok("aarch64-apple-darwin")
        );
        assert_eq!(
            target_triple("x86_64", "macos", env),
            Ok("x86_64-apple-darwin")
        );
        assert_eq!(
            target_triple("x86_64", "linux", env),
            Ok("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            target_triple("aarch64", "linux", env),
            Ok("aarch64-unknown-linux-gnu")
        );
    }
}

#[test]
fn rejects_unsupported_platforms() {
    assert_eq!(
        target_triple("x86_64", "windows", ""),
        Err("no ralph release is built for x86_64 windows".to_string())
    );
    assert_eq!(
        target_triple("riscv64", "linux", "gnu"),
        Err("no ralph release is built for riscv64 linux".to_string())
    );
    assert_eq!(
        target_triple("riscv64", "linux", "musl"),
        Err("no ralph release is built for riscv64 linux".to_string())
    );
}

#[test]
fn refuses_musl_linux_targets() {
    for arch in ["x86_64", "aarch64"] {
        assert_eq!(
            target_triple(arch, "linux", "musl"),
            Err(format!(
                "no musl release is published for {arch}-unknown-linux-musl; build ralph from source"
            ))
        );
    }
}

#[test]
fn picks_the_archive_and_checksum_for_the_target() {
    let latest = release(
        "v0.3.0",
        "",
        &[
            "ralph-v0.3.0-x86_64-apple-darwin.tar.gz",
            "ralph-v0.3.0-aarch64-apple-darwin.tar.gz",
            "ralph-v0.3.0-aarch64-apple-darwin.tar.gz.sha256",
        ],
    );
    assert_eq!(
        pick_download(&latest, "aarch64-apple-darwin").unwrap(),
        Download {
            archive: &latest.assets[1],
            checksum: &latest.assets[2],
        }
    );
    let missing_checksum = pick_download(&latest, "x86_64-apple-darwin").unwrap_err();
    assert!(missing_checksum.contains("ralph-v0.3.0-x86_64-apple-darwin.tar.gz.sha256"));
    let missing_target = pick_download(&latest, "x86_64-unknown-linux-gnu").unwrap_err();
    assert!(missing_target.contains("v0.3.0 has no ralph-v0.3.0-x86_64-unknown-linux-gnu.tar.gz"));
}

#[test]
fn accepts_a_matching_shasum_line() {
    let bytes = b"archive bytes";
    let name = "ralph-v0.3.0-aarch64-apple-darwin.tar.gz";
    let line = format!("{}  {name}\n", sha256_hex(bytes));
    assert_eq!(verify_checksum(&line, name, bytes), Ok(()));
    let upper_binary_mode = format!("{} *{name}", sha256_hex(bytes).to_uppercase());
    assert_eq!(verify_checksum(&upper_binary_mode, name, bytes), Ok(()));
}

#[test]
fn rejects_bad_checksums() {
    let bytes = b"archive bytes";
    let name = "ralph-v0.3.0-aarch64-apple-darwin.tar.gz";
    let good = sha256_hex(bytes);
    let mismatch = verify_checksum(&format!("{}  {name}", sha256_hex(b"other")), name, bytes);
    assert!(mismatch.unwrap_err().contains("checksum mismatch"));
    let wrong_name = verify_checksum(&format!("{good}  other.tar.gz"), name, bytes);
    assert!(wrong_name.unwrap_err().contains("other.tar.gz"));
    for malformed in [
        "",
        "zz  x",
        &good[..63],
        &format!("{}g  {name}", &good[..63]),
    ] {
        assert!(
            verify_checksum(malformed, name, bytes).is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn extracts_the_ralph_entry_only() {
    let bytes = archive(&[("README", b"ignored"), ("ralph", b"\x7fELF binary")]);
    assert_eq!(extract_binary(&bytes).unwrap(), b"\x7fELF binary");
}

#[test]
fn rejects_archives_without_exactly_one_binary() {
    let none = archive(&[("bin/ralph", b"nested")]);
    assert!(extract_binary(&none)
        .unwrap_err()
        .contains("no ralph binary"));
    let twice = archive(&[("ralph", b"one"), ("ralph", b"two")]);
    assert!(extract_binary(&twice)
        .unwrap_err()
        .contains("more than one"));
    assert!(extract_binary(b"not gzip").is_err());
}
