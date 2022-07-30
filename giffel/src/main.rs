mod colorspace;
mod dither;
mod error;
mod image;
mod palette;

use std::path::PathBuf;

use clap::Parser;

use ::image::{DynamicImage, RgbImage};
use error::Error;

use crate::{colorspace::Srgb, image::Image};

#[derive(Parser)]
struct Args {
    image: PathBuf,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    eprintln!("loading image");
    let image = ::image::open(&args.image)?.to_rgba8();

    eprintln!("converting to oklab");
    let image = Image {
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

    // Extract 253 colors, reserving 3 for pure black, pure white, and transparency (index 0.)
    // Note that transparency is not handled in the quantization and dithering processes.
    let mut palette = palette::extract_palette(&image, 253, 16);
    palette.push(
        Srgb {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        }
        .to_linear()
        .to_oklab(),
    );
    palette.push(
        Srgb {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        }
        .to_linear()
        .to_oklab(),
    );
    eprintln!("{palette:?}");
    let palette_image = palette
        .iter()
        .flat_map(|color| color.to_linear().to_srgb().to_array())
        .collect();
    let palette_image = RgbImage::from_vec(palette.len() as u32, 1, palette_image).unwrap();
    DynamicImage::from(palette_image)
        .save("/tmp/palette2.png")
        .unwrap();

    let quantized: Vec<_> = dither::dither(&image, &palette, 0.05)
        .into_iter()
        .flat_map(|index| palette[index as usize].to_linear().to_srgb().to_array())
        .collect();
    let quantized = RgbImage::from_vec(image.width as u32, image.height as u32, quantized).unwrap();
    DynamicImage::from(quantized)
        .save("/tmp/quantized.png")
        .unwrap();

    Ok(())
}
