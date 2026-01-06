use crate::cli::InstallBinaryArgs;
use color_eyre::eyre::{eyre, Context, Result};
use duct::cmd;
use std::env;
use std::fs;
use std::path::PathBuf;

const REPO: &str = "cloudbridgeuy/mcptools";
const BINARY_NAME: &str = "mcptools";

/// Colors for terminal output
struct Colors;

impl Colors {
    fn info(msg: &str) -> String {
        format!("\x1b[0;34mINFO:\x1b[0m {msg}")
    }

    fn success(msg: &str) -> String {
        format!("\x1b[0;32mSUCCESS:\x1b[0m {msg}")
    }

    fn warning(msg: &str) -> String {
        format!("\x1b[1;33mWARNING:\x1b[0m {msg}")
    }
}

fn detect_platform() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let platform = match (os, arch) {
        ("linux", "x86_64") => "Linux-x86_64",
        ("linux", "aarch64") => "Linux-aarch64",
        ("macos", "x86_64") => "Darwin-x86_64",
        ("macos", "aarch64") => "Darwin-arm64",
        ("windows", "x86_64") => "Windows-x86_64",
        _ => {
            return Err(eyre!("Unsupported platform: {} {}", os, arch));
        }
    };

    Ok(platform.to_string())
}

fn get_latest_version() -> Result<String> {
    println!("{}", Colors::info("Fetching latest release information..."));

    let output = cmd!("gh", "api", format!("repos/{}/releases/latest", REPO))
        .read()
        .context(
            "Failed to fetch release information. Make sure gh CLI is installed and authenticated.",
        )?;

    let json: serde_json::Value =
        serde_json::from_str(&output).context("Failed to parse release information")?;

    let tag_name = json["tag_name"]
        .as_str()
        .ok_or_else(|| eyre!("Could not find tag_name in release"))?;

    Ok(tag_name.to_string())
}

fn get_current_version() -> Option<String> {
    cmd!(BINARY_NAME, "--version")
        .read()
        .ok()
        .and_then(|output| output.split_whitespace().nth(1).map(|v| v.to_string()))
}

fn download_binary(version: &str, platform: &str) -> Result<PathBuf> {
    let temp_dir = env::temp_dir();
    let filename = if platform.starts_with("Windows") {
        format!("{BINARY_NAME}-{platform}.exe")
    } else {
        format!("{BINARY_NAME}-{platform}")
    };

    let temp_file = temp_dir.join(&filename);
    let download_url = format!("https://github.com/{REPO}/releases/download/{version}/{filename}");

    println!(
        "{}",
        Colors::info(&format!(
            "Downloading {BINARY_NAME} {version} for {platform}..."
        ))
    );

    // Use curl to download
    cmd!("curl", "-L", &download_url, "-o", &temp_file)
        .run()
        .context(format!("Failed to download binary from {download_url}"))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&temp_file)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&temp_file, perms)?;
    }

    Ok(temp_file)
}

fn install_binary_to_path(temp_file: &PathBuf, install_dir: &PathBuf) -> Result<()> {
    // Create install directory if it doesn't exist
    if !install_dir.exists() {
        println!(
            "{}",
            Colors::info(&format!(
                "Creating install directory: {}",
                install_dir.display()
            ))
        );
        fs::create_dir_all(install_dir)
            .context("Failed to create install directory. You may need to run with sudo.")?;
    }

    let install_path = install_dir.join(BINARY_NAME);

    println!(
        "{}",
        Colors::info(&format!(
            "Installing {} to {}...",
            BINARY_NAME,
            install_path.display()
        ))
    );

    // Copy binary to install path
    fs::copy(temp_file, &install_path)
        .context("Failed to install binary. You may need to run with sudo.")?;

    // Make sure it's executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&install_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&install_path, perms)?;
    }

    println!(
        "{}",
        Colors::success(&format!(
            "{} installed successfully to {}",
            BINARY_NAME,
            install_path.display()
        ))
    );

    Ok(())
}

fn verify_installation() -> Result<()> {
    if cmd!("which", BINARY_NAME).run().is_ok() {
        if let Some(version) = get_current_version() {
            println!(
                "{}",
                Colors::success(&format!(
                    "Installation verified! {BINARY_NAME} version: {version}"
                ))
            );
            println!(
                "{}",
                Colors::info(&format!("Run '{BINARY_NAME} --help' to get started."))
            );
            return Ok(());
        }
    }

    println!(
        "{}",
        Colors::warning("Binary installed but not found in PATH.")
    );
    println!(
        "{}",
        Colors::info("Make sure the install directory is in your PATH environment variable.")
    );

    Ok(())
}

pub fn install_binary(args: &InstallBinaryArgs) -> Result<()> {
    println!(
        "{}",
        Colors::info("MCPTOOLS DevOps CLI Installation Script")
    );
    println!();

    // Detect platform
    let platform = detect_platform()?;
    println!(
        "{}",
        Colors::info(&format!("Detected platform: {platform}"))
    );

    // Get version (latest or specified)
    let version = if let Some(v) = &args.version {
        v.clone()
    } else {
        get_latest_version()?
    };

    println!("{}", Colors::info(&format!("Target version: {version}")));

    // Check if already installed
    if let Some(current_version) = get_current_version() {
        println!(
            "{}",
            Colors::info(&format!("Current installed version: {current_version}"))
        );

        let version_without_v = version.trim_start_matches('v');
        if current_version == version_without_v {
            println!("{}", Colors::info("Latest version already installed!"));
            return Ok(());
        }
    }

    // Download binary
    let temp_file = download_binary(&version, &platform)?;

    // Determine install directory
    let install_dir = if let Some(dir) = &args.install_dir {
        PathBuf::from(dir)
    } else {
        // Default to /usr/local/bin if writable, otherwise ~/.local/bin
        let usr_local_bin = PathBuf::from("/usr/local/bin");
        if usr_local_bin.exists()
            && fs::metadata(&usr_local_bin)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
        {
            usr_local_bin
        } else {
            let home = env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .context("Could not determine home directory")?;
            PathBuf::from(home).join(".local").join("bin")
        }
    };

    // Install binary
    install_binary_to_path(&temp_file, &install_dir)?;

    // Clean up temp file
    let _ = fs::remove_file(&temp_file);

    // Verify installation
    verify_installation()?;

    println!();
    println!(
        "{}",
        Colors::success("Installation completed successfully!")
    );

    Ok(())
}
