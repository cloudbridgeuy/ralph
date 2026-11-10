use chrono::{DateTime, Local, Utc};
use clap::{Subcommand, ValueEnum};
use ralph_core::worker_costs::usage_report::{parse_bound, Dim};
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct UsageArgs {
    #[command(subcommand)]
    pub command: Option<UsageCommand>,
    #[command(flatten)]
    pub filters: UsageFilters,
    #[arg(
        long,
        global = true,
        conflicts_with = "png",
        help = "Print text even on a terminal that can draw the chart"
    )]
    pub text: bool,
    #[arg(
        long,
        global = true,
        value_name = "FILE",
        help = "Read this usage ledger instead of the configured one; a relative path is read from the current directory"
    )]
    pub ledger: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Write the chart to this PNG file instead of printing"
    )]
    pub png: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
pub struct UsageFilters {
    #[arg(
        long,
        global = true,
        value_parser = bound,
        value_name = "DAY|TIME",
        help = "Only rows at or after this local day (YYYY-MM-DD) or RFC 3339 time"
    )]
    pub since: Option<DateTime<Utc>>,
    #[arg(
        long,
        global = true,
        value_parser = bound,
        value_name = "DAY|TIME",
        help = "Only rows before this local day (YYYY-MM-DD) or RFC 3339 time"
    )]
    pub until: Option<DateTime<Utc>>,
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        help = "Chat store: only its rows, and its workers log for tickets"
    )]
    pub store: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Only this harness (main, nap, ask, subagent, claude, cursor, ...)"
    )]
    pub harness: Option<String>,
    #[arg(long, global = true, help = "Only this model")]
    pub model: Option<String>,
    #[arg(long, global = true, help = "Only this ticket")]
    pub ticket: Option<String>,
    #[arg(
        long,
        global = true,
        help = "Only this kind (main, nap, merge, ask, subagent, worker)"
    )]
    pub kind: Option<String>,
}

#[derive(Subcommand, Debug, Clone, Copy)]
pub enum UsageCommand {
    #[command(about = "Spend per local day with a trend line.")]
    Daily,
    #[command(about = "Spend grouped by kind, store, harness, model, ticket or run.")]
    By {
        #[arg(value_enum)]
        dim: UsageBy,
    },
    #[command(about = "Cache hit rate per model.")]
    Cache,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum UsageBy {
    Kind,
    Store,
    Harness,
    Model,
    Ticket,
    Run,
}

fn bound(text: &str) -> Result<DateTime<Utc>, String> {
    parse_bound(text, &Local)
        .ok_or_else(|| format!("`{text}` is not YYYY-MM-DD or an RFC 3339 time"))
}

impl From<UsageBy> for Dim {
    fn from(by: UsageBy) -> Dim {
        match by {
            UsageBy::Kind => Dim::Kind,
            UsageBy::Store => Dim::Store,
            UsageBy::Harness => Dim::Harness,
            UsageBy::Model => Dim::Model,
            UsageBy::Ticket => Dim::Ticket,
            UsageBy::Run => Dim::Run,
        }
    }
}

#[cfg(test)]
mod tests;
