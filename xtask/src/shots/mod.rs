use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

mod browser;
mod core;
mod lines;
mod mask;
mod prefs;
mod report;
mod seed;
mod seed_spec;
mod webdriver;

#[derive(Debug, clap::Parser)]
pub struct ShotsCommand {
    #[command(subcommand)]
    pub action: ShotsAction,
}

#[derive(Debug, clap::Subcommand)]
pub enum ShotsAction {
    List(ListArgs),
    Diff(DiffArgs),
    Compare(CompareArgs),
    Approve(ApproveArgs),
    Seed(SeedArgs),
    Run(RunArgs),
    Check(CheckArgs),
}

#[derive(Debug, clap::Args)]
pub struct ListArgs {
    #[arg(long)]
    pub screen: Option<core::Screen>,

    #[arg(long)]
    pub theme: Option<core::Theme>,
}

#[derive(Debug, clap::Args)]
pub struct DiffArgs {
    pub baseline: PathBuf,

    pub current: PathBuf,

    #[arg(long, default_value = "diff.png")]
    pub out: PathBuf,

    #[arg(long, default_value_t = core::DEFAULT_TOLERANCE)]
    pub tolerance: u8,
}

#[derive(Debug, clap::Args)]
pub struct CompareArgs {
    #[arg(long)]
    pub baseline: PathBuf,

    #[arg(long)]
    pub current: PathBuf,

    #[arg(long)]
    pub out: PathBuf,

    #[arg(long, default_value_t = core::DEFAULT_TOLERANCE)]
    pub tolerance: u8,
}

#[derive(Debug, clap::Args)]
pub struct ApproveArgs {
    #[arg(long)]
    pub baseline: PathBuf,

    #[arg(long)]
    pub current: PathBuf,

    #[arg(long)]
    pub all: bool,

    pub names: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct SeedArgs {
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Debug, clap::Args)]
pub struct RunArgs {
    #[arg(long)]
    pub root: Option<PathBuf>,

    #[arg(long)]
    pub ralph: Option<PathBuf>,

    #[arg(long)]
    pub firefox: Option<PathBuf>,

    #[arg(long)]
    pub screen: Option<core::Screen>,

    #[arg(long)]
    pub theme: Option<core::Theme>,
}

#[derive(Debug, clap::Args)]
pub struct CheckArgs {
    #[command(flatten)]
    pub run: RunArgs,

    #[arg(long, default_value_t = core::DEFAULT_TOLERANCE)]
    pub tolerance: u8,
}

type BoxError = Box<dyn std::error::Error>;

fn read_png(path: &Path) -> Result<core::Rgba, BoxError> {
    let file =
        File::open(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("{} is too large", path.display()))?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer)?;
    let channels = match frame.color_type {
        png::ColorType::Grayscale => core::Channels::Gray,
        png::ColorType::GrayscaleAlpha => core::Channels::GrayAlpha,
        png::ColorType::Rgb => core::Channels::Rgb,
        png::ColorType::Rgba => core::Channels::Rgba,
        png::ColorType::Indexed => {
            return Err(format!("{} is an unexpanded indexed PNG", path.display()).into())
        }
    };
    Ok(core::Rgba {
        width: frame.width,
        height: frame.height,
        pixels: core::expand_to_rgba(channels, &buffer[..frame.buffer_size()]),
    })
}

fn write_png(path: &Path, image: &core::Rgba) -> Result<(), BoxError> {
    let file = File::create(path)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&image.pixels)?;
    Ok(())
}

fn diff(args: DiffArgs) -> Result<(), BoxError> {
    let baseline = read_png(&args.baseline)?;
    let current = read_png(&args.current)?;
    match core::diff_images(&baseline, &current, args.tolerance) {
        core::DiffResult::SizeMismatch { baseline, current } => Err(format!(
            "size mismatch: baseline {}x{}, current {}x{}",
            baseline.0, baseline.1, current.0, current.1
        )
        .into()),
        core::DiffResult::Compared {
            changed,
            total,
            diff,
        } => {
            write_png(&args.out, &diff)?;
            println!(
                "changed {changed} of {total} pixels ({:.2}%); wrote {}",
                core::changed_percent(changed, total),
                args.out.display()
            );
            Ok(())
        }
    }
}

fn baseline_names(dir: &Path) -> Result<Vec<String>, BoxError> {
    if dir.is_dir() {
        png_names(dir)
    } else {
        Ok(Vec::new())
    }
}

fn png_names(dir: &Path) -> Result<Vec<String>, BoxError> {
    let entries =
        fs::read_dir(dir).map_err(|error| format!("could not read {}: {error}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("could not read an entry of {}: {error}", dir.display()))?;
        if entry.path().is_file() {
            files.push(entry.file_name());
        }
    }
    Ok(core::png_names_from(files)?)
}

fn compare(args: CompareArgs) -> Result<(), BoxError> {
    compare_selected(args, None)
}

fn compare_selected(args: CompareArgs, selected: Option<&[core::Shot]>) -> Result<(), BoxError> {
    let mut baseline_names = baseline_names(&args.baseline)?;
    let mut current_names = png_names(&args.current)?;
    if let Some(shots) = selected {
        baseline_names = core::restrict_names(&baseline_names, shots);
        current_names = core::restrict_names(&current_names, shots);
    }
    let mut measures = BTreeMap::new();
    let mut diffs = BTreeMap::new();
    for name in report::common_names(&baseline_names, &current_names) {
        let baseline = read_png(&args.baseline.join(&name))?;
        let current = read_png(&args.current.join(&name))?;
        let result = core::diff_images(&baseline, &current, args.tolerance);
        measures.insert(name.clone(), report::measure(&result));
        if let core::DiffResult::Compared { diff, .. } = result {
            diffs.insert(name, diff);
        }
    }
    let classification = report::classify(&baseline_names, &current_names, &measures);
    let diff_dir = args.out.join("diff");
    fs::create_dir_all(&diff_dir)?;
    let changed_names: Vec<String> = classification
        .changed
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    for name in core::stale_diffs(&png_names(&diff_dir)?, &changed_names) {
        fs::remove_file(diff_dir.join(name))?;
    }
    for (name, _) in &classification.changed {
        if let Some(diff) = diffs.get(name) {
            write_png(&diff_dir.join(name), diff)?;
        }
    }
    let report = report::render_report(&classification);
    fs::write(args.out.join("report.md"), &report)?;
    print!("{report}");
    if report::is_clean(&classification) {
        Ok(())
    } else {
        Err("shots are not clean".into())
    }
}

const DEFAULT_FIREFOX: &str = "/Applications/Firefox Developer Edition.app/Contents/MacOS/firefox";

const TIMED_OUT_FILE: &str = ".timed-out";

fn read_timed_out(current: &Path) -> Result<Vec<String>, BoxError> {
    match fs::read_to_string(current.join(TIMED_OUT_FILE)) {
        Ok(text) => Ok(report::parse_timed_out(&text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn write_timed_out(current: &Path, names: &[String]) -> Result<(), BoxError> {
    let path = current.join(TIMED_OUT_FILE);
    if names.is_empty() {
        return match fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        };
    }
    fs::write(path, report::render_timed_out(names))?;
    Ok(())
}

fn capture_current(args: RunArgs) -> Result<(PathBuf, Vec<core::Shot>), BoxError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let resolve = |given, under_home| {
        webdriver::resolve_path(given, home.as_deref(), under_home)
            .ok_or_else(|| BoxError::from("HOME is not set; pass the path as a flag"))
    };
    let root = resolve(args.root, ".local/share/ralph-shots")?;
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| BoxError::from("the xtask manifest directory has no parent"))?;
    let target_dir = std::env::var("CARGO_TARGET_DIR").ok();
    let cwd = std::env::current_dir()?;
    let options = browser::CaptureOptions {
        ralph: args
            .ralph
            .unwrap_or_else(|| core::default_ralph_path(repo_root, &cwd, target_dir.as_deref())),
        firefox: args
            .firefox
            .unwrap_or_else(|| PathBuf::from(DEFAULT_FIREFOX)),
        screen: args.screen,
        theme: args.theme,
    };
    let selected = core::filter(core::matrix(), options.screen, options.theme);
    let current = root.join("current");
    let timed_out = browser::capture(&options, &current)?;
    let selected_names: Vec<String> = selected.iter().map(core::Shot::name).collect();
    let merged = report::merge_timed_out(&read_timed_out(&current)?, &selected_names, &timed_out);
    write_timed_out(&current, &merged)?;
    if !timed_out.is_empty() {
        println!("timed out waiting for ready: {}", timed_out.join(", "));
    }
    Ok((root, selected))
}

fn check(args: CheckArgs) -> Result<(), BoxError> {
    let filtered = args.run.screen.is_some() || args.run.theme.is_some();
    let (root, selected) = capture_current(args.run)?;
    let compare_args = CompareArgs {
        baseline: root.join("baseline"),
        current: root.join("current"),
        out: root.join("report"),
        tolerance: args.tolerance,
    };
    compare_selected(compare_args, filtered.then_some(selected.as_slice()))
}

fn approve(args: ApproveArgs) -> Result<(), BoxError> {
    let current_names = png_names(&args.current)?;
    let timed_out = read_timed_out(&args.current)?;
    let listed: Vec<String> = core::matrix().iter().map(core::Shot::name).collect();
    let selection =
        report::select_approvals(&args.names, args.all, &current_names, &listed, &timed_out)?;
    for name in &selection.warned {
        println!("warning: {name} timed out waiting for ready; approving it as named");
    }
    for name in &selection.skipped {
        println!("skipped {name}: not in the shot list");
    }
    let approved = selection.approved;
    for name in &approved {
        let image = read_png(&args.current.join(name))?;
        report::require_image_size(name, image.width, image.height)?;
    }
    fs::create_dir_all(&args.baseline)?;
    for name in &approved {
        fs::copy(args.current.join(name), args.baseline.join(name))?;
        println!("approved {name}");
    }
    Ok(())
}

pub fn run(command: ShotsCommand) -> Result<(), BoxError> {
    match command.action {
        ShotsAction::List(args) => {
            for shot in core::filter(core::matrix(), args.screen, args.theme) {
                println!("{}", shot.name());
            }
            Ok(())
        }
        ShotsAction::Diff(args) => diff(args),
        ShotsAction::Compare(args) => compare(args),
        ShotsAction::Approve(args) => approve(args),
        ShotsAction::Run(args) => capture_current(args).map(|_| ()),
        ShotsAction::Check(args) => check(args),
        ShotsAction::Seed(args) => {
            let written = seed::seed(&args.out)?;
            println!("seeded {written} records into {}", args.out.display());
            Ok(())
        }
    }
}
