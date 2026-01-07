//! See <https://github.com/matklad/cargo-xtask/>
//!
//! This binary defines various auxiliary build commands, which are not
//! expressible with just `cargo`.
//!
//! The binary is integrated into the `cargo` command line by using an
//! alias in `.cargo/config`.

use clap::Parser;

mod lint;
mod prelude;

/// Development tasks for the calendsync repository
#[derive(Debug, Parser)]
#[command(name = "xtask")]
#[command(about = "Development tasks for calendsync", long_about = None)]
struct Cli {
    #[command(flatten)]
    global: Global,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Global {
    /// Silence the command output
    #[clap(long, global = true)]
    pub silent: bool,

    /// Enable verbose output
    #[clap(long, global = true)]
    pub verbose: bool,
}

impl Global {
    pub fn is_silent(&self) -> bool {
        self.silent
    }

    pub fn is_verbose(&self) -> bool {
        self.verbose
    }
}

#[derive(Debug, clap::Subcommand)]
enum Commands {
    /// Code quality checks and git hooks management
    Lint(lint::LintCommand),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Lint(lint_cmd) => {
            lint::run(lint_cmd, cli.global).await?;
        }
    }

    Ok(())
}
