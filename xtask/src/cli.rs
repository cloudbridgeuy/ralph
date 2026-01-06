use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "xtasks")]
#[command(about = "Run project tasks using rust instead of scripts")]
pub struct App {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Builds a binary and installs it at the given path
    Install(InstallArgs),
    /// Manage git hooks (install, uninstall, status, test)
    Hooks(HooksArgs),
    /// Create and manage releases
    Release(ReleaseArgs),
    /// Download and install binary from GitHub releases
    InstallBinary(InstallBinaryArgs),
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Name of the binary to install (defaults to "mcptools")
    #[arg(short, long, default_value = "mcptools")]
    pub name: String,

    /// Directory to install the binary to (defaults to ~/.local/bin)
    #[arg(short, long)]
    pub path: Option<String>,
}

#[derive(Args, Debug)]
pub struct HooksArgs {
    #[command(subcommand)]
    pub command: HooksCommands,
}

#[derive(Debug, Subcommand)]
pub enum HooksCommands {
    /// Install git hooks
    Install,
    /// Uninstall git hooks
    Uninstall,
    /// Show git hooks status
    Status,
    /// Test git hooks
    Test,
}

#[derive(Args, Debug)]
pub struct ReleaseArgs {
    /// Version to release (e.g., 1.0.0, 2.1.0-beta.1)
    pub version: Option<String>,

    /// Clean up a failed release tag
    #[arg(long)]
    pub cleanup: Option<String>,

    /// Automatically upgrade local binary after successful release
    #[arg(long)]
    pub auto_upgrade: bool,

    /// Skip workflow monitoring
    #[arg(long)]
    pub no_monitor: bool,
}

#[derive(Args, Debug)]
pub struct InstallBinaryArgs {
    /// Installation directory (defaults to /usr/local/bin or ~/.local/bin)
    #[arg(short = 'd', long)]
    pub install_dir: Option<String>,

    /// Specific version to install (defaults to latest)
    #[arg(short, long)]
    pub version: Option<String>,
}
