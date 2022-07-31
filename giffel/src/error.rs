use image::ImageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Error while loading image: {0}")]
    Image(#[from] ImageError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Palette must not be larger than 256 colors")]
    PaletteTooBig,
    #[error("Palette is empty")]
    PaletteIsEmpty,
    #[error(
        "Frame is incompatible with this archive (dimensions and palette color count differs)"
    )]
    FrameIncompatible,
}
