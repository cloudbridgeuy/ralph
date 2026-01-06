use crate::cli;
use color_eyre::eyre::Result;
use duct::cmd;
use std::env;
use std::path::PathBuf;

pub fn install(args: &cli::InstallArgs) -> Result<()> {
    println!("Building {} in release mode...", args.name);

    // Build the binary for the current target
    cmd!("cargo", "build", "--bin", &args.name, "--release").run()?;

    // Determine install path
    let install_dir = if let Some(path) = &args.path {
        PathBuf::from(path)
    } else {
        // Default to ~/.local/bin
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .expect("Could not determine home directory");
        PathBuf::from(home).join(".local").join("bin")
    };

    // Create the directory if it doesn't exist
    if !install_dir.exists() {
        println!("Creating directory: {}", install_dir.display());
        std::fs::create_dir_all(&install_dir)?;
    }

    // Source and destination paths
    let binary_name = &args.name;
    let source_path = PathBuf::from("target").join("release").join(binary_name);
    let dest_path = install_dir.join(binary_name);

    println!("Installing {} to {}", binary_name, dest_path.display());

    // Copy the binary
    std::fs::copy(&source_path, &dest_path)?;

    // Make it executable (Unix-like systems)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest_path, perms)?;
    }

    // Fix macOS code signing issues
    #[cfg(target_os = "macos")]
    {
        // Remove all extended attributes
        let _ = cmd!("xattr", "-cr", &dest_path).run();

        // Re-sign with ad-hoc signature to fix "killed" errors
        if let Err(e) = cmd!("codesign", "--force", "--sign", "-", &dest_path).run() {
            eprintln!("Warning: Failed to re-sign binary: {e}");
            eprintln!(
                "You may need to run: codesign --force --sign - {}",
                dest_path.display()
            );
        }
    }

    println!(
        "✓ Successfully installed {} to {}",
        binary_name,
        dest_path.display()
    );

    // Check if the install directory is in PATH
    if let Ok(path_var) = env::var("PATH") {
        let install_dir_str = install_dir.to_string_lossy();
        if !path_var.split(':').any(|p| p == install_dir_str) {
            println!("\nNote: {} is not in your PATH.", install_dir.display());
            println!("Add it to your PATH by adding this line to your shell config:");
            println!("  export PATH=\"{}:$PATH\"", install_dir.display());
        }
    }

    Ok(())
}
