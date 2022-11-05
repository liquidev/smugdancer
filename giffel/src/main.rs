mod archive;
mod colorspace;
mod crop;
mod dither;
mod error;
mod image;
mod palette;

use std::{
    borrow::Cow,
    fs::File,
    io::{Stderr, Write},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use gif::DisposalMethod;
use parking_lot::Mutex;
use pbr::ProgressBar;
use rayon::prelude::*;

use crate::{
    crop::{crop, find_opaque_frame},
    image::Image,
};
use archive::{ArchiveReader, ArchiveWriter};
use colorspace::Oklab;
use colorspace::Srgb;
use dither::dither;
use error::Error;
use palette::extract_palette;

/// A specialized GIF encoder whose main goal is being able to stitch selected frames
/// into one GIF very fast.
#[derive(Parser)]
#[clap(author, version)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new giffel archive from the provided image files.
    Archive(ArchiveCommand),
    /// Stitch frames from an archive into a GIF.
    Stitch(StitchCommand),
    /// Return stats about an archive.
    Stat(StatCommand),
}

#[derive(Args)]
struct ArchiveCommand {
    /// The image files to pack into the archive.
    images: Vec<PathBuf>,
    /// The output archive filename. Giffel archives usually use the extension `.giffel`.
    #[clap(short, long)]
    output: PathBuf,
    /// Disable sorting of filenames.
    #[clap(long)]
    no_sort: bool,
}

#[derive(Args)]
struct StitchCommand {
    /// The archive to use.
    #[clap(short, long)]
    archive: PathBuf,
    /// Which frames to use from the archive. Note that frame indices start at 1.
    frames: Vec<usize>,
    /// Output path. Set to `-` for stdout.
    #[clap(short, long)]
    output: String,
    /// The framerate to encode the GIF with. Note that not all values are valid; only framerates
    /// coming from multiples of 10ms, greater than 20ms are supported (50 fps is the limit.)
    #[clap(short = 'r', long, default_value = "25")]
    fps: u32,
}

#[derive(Subcommand)]
enum StatTarget {
    /// Get the width of the image stored in the archive.
    Width,
    /// Get the height of the image stored in the archive.
    Height,
    /// Get the number of images stored in the archive.
    FrameCount,
}

#[derive(Args)]
struct StatCommand {
    /// The archive to stat.
    archive: PathBuf,
    /// What to stat.
    #[clap(subcommand)]
    target: StatTarget,
}

fn progress_bar(max: u64) -> ProgressBar<Stderr> {
    let stderr = std::io::stderr();
    ProgressBar::on(stderr, max)
}

fn load_oklab_alpha_image(path: PathBuf) -> Result<(Image<Oklab>, Image<u8>), Error> {
    let image = ::image::open(path)?.to_rgba8();

    let oklab = Image {
        width: image.width() as usize,
        height: image.height() as usize,
        pixels: image
            .chunks(4)
            .map(|color| {
                Srgb::from_array([color[0], color[1], color[2]])
                    .to_linear()
                    .to_oklab()
            })
            .collect(),
    };
    let alpha = Image {
        width: image.width() as usize,
        height: image.height() as usize,
        pixels: image.chunks(4).map(|color| color[3]).collect(),
    };

    Ok((oklab, alpha))
}

fn archive(mut command: ArchiveCommand) -> Result<(), Error> {
    if !command.no_sort {
        command.images.sort_by(|a, b| {
            'try_parse_number: {
                let (Some(a_stem), Some(b_stem)) = (a.file_stem(), b.file_stem())
                else { break 'try_parse_number };
                let (Some(a_str), Some(b_str)) = (a_stem.to_str(), b_stem.to_str())
                else { break 'try_parse_number };
                let (Ok(x), Ok(y)) = (a_str.parse::<usize>(), b_str.parse::<usize>())
                else { break 'try_parse_number };
                return x.cmp(&y);
            }
            a.cmp(b)
        });
    }
    let images: Vec<_> = command
        .images
        .into_iter()
        .flat_map(|path| {
            if path.is_dir() {
                eprintln!("reading all files from input directory {path:?}");
                let iter = match std::fs::read_dir(path) {
                    Ok(iter) => iter,
                    Err(error) => {
                        eprintln!("cannot read input directory: {error}");
                        return vec![];
                    }
                };
                iter.flat_map(|result| result.ok())
                    .map(|entry| entry.path())
                    .collect()
            } else {
                vec![path]
            }
        })
        .collect();
    eprintln!("preparing images, this will take a while!");

    let frame_count = images.len();
    let progress = Arc::new(Mutex::new(progress_bar(frame_count as u64)));
    progress
        .lock()
        .set_max_refresh_rate(Some(Duration::from_millis(20)));
    let frames: Vec<_> = images
        .into_par_iter()
        .map({
            let progress = Arc::clone(&progress);
            move |path| {
                let (oklab, alpha) = load_oklab_alpha_image(path).expect("cannot load image");

                // NOTE: Generate 253 colors, leaving three free slots for pure black, pure white,
                // and transparency.
                let mut palette = extract_palette(&oklab, 253, 16);
                palette.push(Oklab::WHITE);
                palette.push(Oklab::BLACK);

                let mut indexed = dither(&oklab, &palette, 0.05);

                let transparent = palette.len() as u8;
                palette.push(Oklab::BLACK); // transparent

                for y in 0..indexed.height {
                    for x in 0..indexed.width {
                        if alpha[(x, y)] < 128 {
                            indexed[(x, y)] = transparent;
                        }
                    }
                }

                let palette: Vec<_> = palette
                    .iter()
                    .map(|oklab| oklab.to_linear().to_srgb().to_array())
                    .collect();
                progress.lock().inc();
                (indexed, palette)
            }
        })
        .collect();

    eprintln!("writing archive");
    let mut progress = progress_bar(frame_count as u64);
    let mut archive = ArchiveWriter::new(File::create(command.output)?);
    for (image, palette) in frames {
        archive.write_frame(&image, &palette)?;
        progress.inc();
    }

    Ok(())
}

fn stitch(command: StitchCommand) -> Result<(), Error> {
    eprintln!("reading archive");
    let mut archive = ArchiveReader::new(File::open(command.archive)?)?;
    eprintln!("{:?}", archive.dimensions);

    let frame_count = command.frames.len();
    if frame_count == 0 {
        return Err(Error::EmptyGif);
    }

    let mut progress = progress_bar(frame_count as u64);
    let frames: Vec<_> = command
        .frames
        .iter()
        .map(|&index| {
            let (image, palette) = archive.read_frame(index).expect("cannot read frame");
            progress.inc();
            (image, palette)
        })
        .map(|(image, palette)| {
            let bounds = find_opaque_frame(&image);
            let image = crop(&image, &bounds);
            (image, palette, bounds)
        })
        .collect();

    let writer: Box<dyn Write> = if command.output == "-" {
        Box::new(std::io::stdout())
    } else {
        Box::new(File::create(command.output)?)
    };

    eprintln!("encoding frames");
    let mut progress = progress_bar(frames.len() as u64);
    let mut encoder = gif::Encoder::new(
        writer,
        archive.dimensions.width,
        archive.dimensions.height,
        &[],
    )?;
    encoder.set_repeat(gif::Repeat::Infinite)?;
    let delay = u16::try_from(100 / command.fps).map_err(|_| Error::InvalidFramerate)?;
    for (image, palette, rect) in frames {
        let frame = gif::Frame {
            delay,
            dispose: DisposalMethod::Background,
            transparent: Some(255),
            left: rect.x as u16,
            top: rect.y as u16,
            width: rect.width as u16,
            height: rect.height as u16,
            palette: Some(palette.iter().copied().flatten().collect()),
            buffer: Cow::Borrowed(&image.pixels),
            interlaced: false,
            needs_user_input: false,
        };
        encoder.write_frame(&frame)?;
        progress.inc();
    }
    eprintln!("writing trailer");
    let _writer = encoder.into_inner();

    Ok(())
}

fn stat(command: StatCommand) -> Result<(), Error> {
    let archive = File::open(&command.archive)?;
    let reader = ArchiveReader::new(archive)?;

    match command.target {
        StatTarget::Width => println!("{}", reader.dimensions.width),
        StatTarget::Height => println!("{}", reader.dimensions.height),
        StatTarget::FrameCount => println!("{}", reader.frame_count),
    }

    Ok(())
}

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    match args.command {
        Command::Archive(cmd) => archive(cmd)?,
        Command::Stitch(cmd) => stitch(cmd)?,
        Command::Stat(cmd) => stat(cmd)?,
    }

    Ok(())
}
