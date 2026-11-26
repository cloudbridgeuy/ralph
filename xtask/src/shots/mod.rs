use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

mod core;

#[derive(Debug, clap::Parser)]
pub struct ShotsCommand {
    #[command(subcommand)]
    pub action: ShotsAction,
}

#[derive(Debug, clap::Subcommand)]
pub enum ShotsAction {
    List(ListArgs),
    Diff(DiffArgs),
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

    #[arg(long, default_value_t = 8)]
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

pub fn run(command: ShotsCommand) -> Result<(), BoxError> {
    match command.action {
        ShotsAction::List(args) => {
            for shot in core::filter(core::matrix(), args.screen, args.theme) {
                println!("{}", shot.name());
            }
            Ok(())
        }
        ShotsAction::Diff(args) => diff(args),
    }
}
