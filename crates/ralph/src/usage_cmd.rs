use crate::chat::session::store_dir;
use crate::cli::{UsageArgs, UsageCommand};
use crate::render::image::{detect, draw_png, ImageBounds};
use crate::render::svg_picture::rasterize_svg;
use chrono::Local;
use ralph_core::wire::provider::{provider, ProviderId};
use ralph_core::wire::view::Pricing;
use ralph_core::worker::workers_log_path;
use ralph_core::worker_costs::usage_report::text::{chart, choose, render, Inputs, Output, View};
use ralph_core::worker_costs::usage_report::{Dim, Filter};
use ralph_server::chat::memory::{read_optional, usage_path};
use ralph_server::config::AppConfig;
use std::error::Error;
use std::io::{ErrorKind, IsTerminal};
use std::{fs, path::Path};

fn view_of(command: Option<UsageCommand>) -> View {
    match command {
        None | Some(UsageCommand::Daily) => View::Daily,
        Some(UsageCommand::By { dim }) => View::By(Dim::from(dim)),
        Some(UsageCommand::Cache) => View::Cache,
    }
}

fn read_ledger(path: &Path) -> Result<Option<String>, Box<dyn Error>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display()).into()),
    }
}

pub fn run(args: UsageArgs) -> Result<(), Box<dyn Error>> {
    let path = usage_path();
    let Some(usage) = read_ledger(&path)? else {
        println!("No usage ledger at {}.", path.display());
        return Ok(());
    };
    let filters = args.filters;
    let store = store_dir(filters.store.clone())?;
    let workers = String::from_utf8_lossy(&read_optional(&workers_log_path(&store))?).into_owned();
    let config = AppConfig::load()?;
    let provider = provider(ProviderId::Anthropic);
    let pricing = Pricing {
        provider: provider.as_ref(),
        overrides: &config.wire.prices,
    };
    let filter = Filter {
        since: filters.since,
        until: filters.until,
        store: filters.store.map(|_| store.display().to_string()),
        harness: filters.harness,
        model: filters.model,
        ticket: filters.ticket,
        kind: filters.kind,
    };
    let inputs = Inputs {
        usage: &usage,
        workers: &workers,
    };
    let view = view_of(args.command);
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let mut settings = None;
    let output = choose(view, args.text, args.png, || {
        settings = detect(color);
        settings.is_some()
    })?;
    let picture = |svg: Option<String>| -> Result<Vec<u8>, Box<dyn Error>> {
        let svg = svg.ok_or("the view has no chart")?;
        Ok(rasterize_svg(&svg)?)
    };
    match output {
        Output::Png(path) => {
            fs::write(
                &path,
                picture(chart(view, &inputs, &filter, &Local, &pricing))?,
            )?;
            println!("Wrote {}", path.display());
        }
        Output::Inline => {
            let bytes = picture(chart(view, &inputs, &filter, &Local, &pricing))?;
            match settings.and_then(|s| draw_png(&bytes, s, ImageBounds::current())) {
                Some(drawn) => println!("{drawn}"),
                None => println!("{}", render(view, &inputs, &filter, &Local, &pricing)),
            }
        }
        Output::Text => println!("{}", render(view, &inputs, &filter, &Local, &pricing)),
    }
    Ok(())
}
