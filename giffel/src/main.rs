mod archive;
mod colorspace;
mod dither;
mod error;
mod image;
mod palette;

use std::{fs::File, io::Stderr, path::PathBuf, sync::Arc, time::Duration};

use clap::{Args, Parser, Subcommand};

use ::image::{DynamicImage, RgbImage};
use colorspace::Oklab;
use error::Error;
use palette::extract_palette;
use parking_lot::Mutex;
use pbr::ProgressBar;
use rayon::prelude::*;

use crate::{
    archive::{ArchiveReader, ArchiveWriter},
    colorspace::Srgb,
    dither::dither,
    image::Image,
};

/// Giffel is a specialized GIF encoder whose main goal is being able to stitch selected frames
/// into one GIF very fast.
#[derive(Parser)]
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
        command.images.sort();
    }

    eprintln!("preparing images, this will take a while!");

    let frame_count = command.images.len();
    let progress = Arc::new(Mutex::new(progress_bar(frame_count as u64)));
    progress
        .lock()
        .set_max_refresh_rate(Some(Duration::from_millis(20)));
    let frames: Vec<_> = command
        .images
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
                        if alpha[(x, y)] < 32 {
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

    Ok(())
}

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    match args.command {
        Command::Archive(cmd) => archive(cmd)?,
        Command::Stitch(cmd) => stitch(cmd)?,
    }

    // eprintln!("loading image");
    // let image = ::image::open(&args.image)?.to_rgba8();

    // eprintln!("converting to oklab");
    // let image = Image {
    //     width: image.width() as usize,
    //     height: image.height() as usize,
    //     pixels: image
    //         .chunks(4)
    //         .map(|color| {
    //             Srgb::from_array([color[0], color[1], color[2]])
    //                 .to_linear()
    //                 .to_oklab()
    //         })
    //         .collect(),
    // };

    // // Extract 253 colors, reserving 3 for pure black, pure white, and transparency (index 0.)
    // // Note that transparency is not handled in the quantization and dithering processes.
    // let mut palette = palette::extract_palette(&image, 253, 16);
    // palette.push(
    //     Srgb {
    //         r: 0.0,
    //         g: 0.0,
    //         b: 0.0,
    //     }
    //     .to_linear()
    //     .to_oklab(),
    // );
    // palette.push(
    //     Srgb {
    //         r: 1.0,
    //         g: 1.0,
    //         b: 1.0,
    //     }
    //     .to_linear()
    //     .to_oklab(),
    // );
    // eprintln!("{palette:?}");
    // let palette_image = palette
    //     .iter()
    //     .flat_map(|color| color.to_linear().to_srgb().to_array())
    //     .collect();
    // let palette_image = RgbImage::from_vec(palette.len() as u32, 1, palette_image).unwrap();
    // DynamicImage::from(palette_image)
    //     .save("/tmp/palette2.png")
    //     .unwrap();

    // let quantized: Vec<_> = dither::dither(&image, &palette, 0.05)
    //     .into_iter()
    //     .flat_map(|index| palette[index as usize].to_linear().to_srgb().to_array())
    //     .collect();
    // let quantized = RgbImage::from_vec(image.width as u32, image.height as u32, quantized).unwrap();
    // DynamicImage::from(quantized)
    //     .save("/tmp/quantized.png")
    //     .unwrap();

    Ok(())
}
