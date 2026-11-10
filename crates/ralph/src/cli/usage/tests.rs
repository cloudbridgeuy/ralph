use crate::cli::{Cli, Commands, UsageArgs, UsageCommand};
use clap::Parser;
use ralph_core::worker_costs::usage_report::Dim;

fn parsed(args: &[&str]) -> Result<UsageArgs, clap::Error> {
    let mut words = vec!["ralph", "usage"];
    words.extend(args);
    Cli::try_parse_from(words).map(|cli| match cli.command {
        Some(Commands::Usage(args)) => args,
        _ => panic!("expected usage"),
    })
}

#[test]
fn bare_usage_parses_with_no_subcommand() {
    let args = parsed(&[]);
    assert!(
        args.as_ref().is_ok_and(|args| args.command.is_none()),
        "{args:?}"
    );
}

#[test]
fn daily_and_cache_parse() {
    let daily = parsed(&["daily"]);
    assert!(matches!(
        daily.map(|a| a.command),
        Ok(Some(UsageCommand::Daily))
    ));
    let cache = parsed(&["cache"]);
    assert!(matches!(
        cache.map(|a| a.command),
        Ok(Some(UsageCommand::Cache))
    ));
}

#[test]
fn by_takes_each_dimension() {
    let cases = [
        ("kind", Dim::Kind),
        ("store", Dim::Store),
        ("harness", Dim::Harness),
        ("model", Dim::Model),
        ("ticket", Dim::Ticket),
        ("run", Dim::Run),
    ];
    for (word, dim) in cases {
        let args = parsed(&["by", word]);
        let got = args.ok().and_then(|args| match args.command {
            Some(UsageCommand::By { dim }) => Some(Dim::from(dim)),
            _ => None,
        });
        assert_eq!(got, Some(dim), "by {word}");
    }
}

#[test]
fn by_rejects_an_unknown_dimension() {
    assert!(parsed(&["by", "colour"]).is_err());
    assert!(parsed(&["by"]).is_err());
}

#[test]
fn the_filters_parse_before_or_after_the_subcommand() {
    let args = parsed(&[
        "by",
        "model",
        "--since",
        "2026-11-01",
        "--until",
        "2026-11-09",
        "--store",
        "/s",
        "--harness",
        "claude",
        "--model",
        "m",
        "--ticket",
        "GUZ-1",
        "--kind",
        "ask",
    ]);
    let ok = args.is_ok_and(|a| {
        a.filters.since.is_some()
            && a.filters.until.is_some()
            && a.filters.store.as_deref() == Some(std::path::Path::new("/s"))
            && a.filters.harness.as_deref() == Some("claude")
            && a.filters.model.as_deref() == Some("m")
            && a.filters.ticket.as_deref() == Some("GUZ-1")
            && a.filters.kind.as_deref() == Some("ask")
    });
    assert!(ok);
    let early = parsed(&["--harness", "claude", "daily"]);
    assert!(early.is_ok_and(|a| a.filters.harness.as_deref() == Some("claude")));
}

#[test]
fn a_bad_since_is_a_validation_error() {
    let error = parsed(&["--since", "yesterday"]).err();
    assert_eq!(
        error.map(|e| e.kind()),
        Some(clap::error::ErrorKind::ValueValidation)
    );
}

#[test]
fn text_and_png_parse_before_or_after_the_subcommand() {
    let text = parsed(&["--text", "daily"]);
    assert!(text.is_ok_and(|a| a.text));
    let png = parsed(&["by", "model", "--png", "/tmp/m.png"]);
    assert!(png.is_ok_and(|a| a.png.as_deref() == Some(std::path::Path::new("/tmp/m.png"))));
    let early = parsed(&["--png", "/tmp/d.png", "daily"]);
    assert!(early.is_ok_and(|a| a.png.is_some()));
}

#[test]
fn text_and_png_conflict() {
    let error = parsed(&["--text", "--png", "/tmp/x.png"]).err();
    assert_eq!(
        error.map(|e| e.kind()),
        Some(clap::error::ErrorKind::ArgumentConflict)
    );
}
