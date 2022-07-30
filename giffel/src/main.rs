mod dither;
mod error;
mod palette;

use std::path::PathBuf;

use clap::Parser;

use error::Error;
use image::{DynamicImage, RgbImage};

#[derive(Parser)]
struct Args {
    image: PathBuf,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    eprintln!("loading image");
    let image = image::open(&args.image)?.to_rgb8();

    // Extract 253 colors, reserving 3 for pure black, pure white, and transparency (index 0.)
    // Note that transparency is not handled in the quantization and dithering processes.
    let mut palette = palette::extract_palette(&image, 253, 16);
    palette.push([0, 0, 0]);
    palette.push([255, 255, 255]);
    eprintln!("{palette:?}");
    let palette_image = palette.iter().copied().flatten().collect();
    let palette_image = RgbImage::from_vec(palette.len() as u32, 1, palette_image).unwrap();
    DynamicImage::from(palette_image)
        .save("/tmp/palette2.png")
        .unwrap();

    let quantized: Vec<_> = dither::dither(&image, &palette, 1.0)
        .into_iter()
        .flat_map(|index| palette[index as usize])
        .collect();
    let quantized = RgbImage::from_vec(image.width(), image.height(), quantized).unwrap();
    DynamicImage::from(quantized)
        .save("/tmp/quantized.png")
        .unwrap();

    Ok(())
}
