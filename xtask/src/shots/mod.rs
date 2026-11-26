mod core;

#[derive(Debug, clap::Parser)]
pub struct ShotsCommand {
    #[command(subcommand)]
    pub action: ShotsAction,
}

#[derive(Debug, clap::Subcommand)]
pub enum ShotsAction {
    List(ListArgs),
}

#[derive(Debug, clap::Args)]
pub struct ListArgs {
    #[arg(long)]
    pub screen: Option<core::Screen>,

    #[arg(long)]
    pub theme: Option<core::Theme>,
}

pub fn run(command: ShotsCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command.action {
        ShotsAction::List(args) => {
            for shot in core::filter(core::matrix(), args.screen, args.theme) {
                println!("{}", shot.name());
            }
        }
    }
    Ok(())
}
