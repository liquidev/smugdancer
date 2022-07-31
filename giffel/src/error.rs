use image::ImageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Error while loading image: {0}")]
    Image(#[from] ImageError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("GIF encoding error: {0}")]
    GifEncode(#[from] gif::EncodingError),

    #[error("Palette must not be larger than 256 colors")]
    PaletteTooBig,
    #[error("Palette is empty")]
    PaletteIsEmpty,
    #[error(
        "Frame is incompatible with this archive (dimensions and palette color count differs)"
    )]
    FrameIncompatible,
    #[error("Frame index {got} is out of bounds ({count} frames are stored in the file)")]
    FrameOutOfBounds { got: usize, count: usize },
    #[error("Frames are too big to encode in a GIF")]
    FramesTooBig,
    #[error("File does not appear to be a giffel archive")]
    InvalidMagic,

    #[error("Invalid framerate supplied (frame delay exceeded 65536 - how?????)")]
    InvalidFramerate,
    #[error("No frames provided")]
    EmptyGif,
}
