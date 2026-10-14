use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .args(args)
        .current_dir(Path::new(&dir))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn watch_git_path(name: &str) {
    if let Some(path) = git(&["rev-parse", "--git-path", name]) {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    for name in ["HEAD", "index", "refs/tags", "packed-refs"] {
        watch_git_path(name);
    }
    if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(&head_ref);
    }

    let sha = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain"]).is_some_and(|status| !status.is_empty());
    let release_tag = format!(
        "v{}",
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    );
    let at_release_tag = git(&["tag", "--points-at", "HEAD"])
        .is_some_and(|tags| tags.lines().any(|tag| tag == release_tag));

    println!("cargo:rustc-env=RALPH_GIT_SHA={sha}");
    println!("cargo:rustc-env=RALPH_GIT_DIRTY={}", u8::from(dirty));
    println!(
        "cargo:rustc-env=RALPH_AT_RELEASE_TAG={}",
        u8::from(at_release_tag)
    );
}
