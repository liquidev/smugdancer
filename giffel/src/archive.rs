//! Support for giffel archive files.

use std::{
    io::{Read, Write},
    mem::size_of,
};

use crate::{error::Error, image::Image};

pub const HEADER_SIZE: usize = size_of::<usize>() * 2 + size_of::<u8>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: usize,
    pub height: usize,
    pub palette_color_count: u8,
}

impl Dimensions {
    fn of(image: &Image<u8>, palette: &[[u8; 3]]) -> Result<Self, Error> {
        if palette.is_empty() {
            return Err(Error::PaletteIsEmpty);
        }
        Ok(Self {
            width: image.width,
            height: image.height,
            palette_color_count: u8::try_from(palette.len() - 1)
                .map_err(|_| Error::PaletteTooBig)?,
        })
    }
}

/// Writer for giffel archive files.
pub struct ArchiveWriter<W> {
    writer: W,
    dimensions: Option<Dimensions>,
}

impl<W> ArchiveWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            dimensions: None,
        }
    }
}

impl<W> ArchiveWriter<W>
where
    W: Write,
{
    fn write_dimensions(&mut self, dims: Dimensions) -> Result<(), Error> {
        self.writer.write_all(&dims.width.to_le_bytes())?;
        self.writer.write_all(&dims.height.to_le_bytes())?;
        self.writer.write_all(&[dims.palette_color_count])?;

        Ok(())
    }

    /// Writes frames to the archive. Each frame is made up of an image and a palette. Colors in the
    /// palette are specified in a slice of `[u8; 3]`, each array is an `[R, G, B]` color. The
    /// color index 255 is treated as transparency.
    ///
    /// Do note that every frame must have the same dimensions and palette color count.
    pub fn write_frame(&mut self, image: &Image<u8>, palette: &[[u8; 3]]) -> Result<(), Error> {
        if self.dimensions.is_none() {
            let dimensions = Dimensions::of(image, palette)?;
            self.write_dimensions(dimensions)?;
            self.dimensions = Some(dimensions);
        }
        if Some(Dimensions::of(image, palette)?) != self.dimensions {
            return Err(Error::FrameIncompatible);
        }

        for color in palette {
            self.writer.write_all(color)?;
        }
        self.writer.write_all(&image.pixels)?;

        Ok(())
    }
}

fn read_bytes<R, const N: usize>(mut reader: R) -> Result<[u8; N], std::io::Error>
where
    R: Read,
{
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub struct ArchiveReader<R> {
    reader: R,
    pub dimensions: Dimensions,
}

impl<R> ArchiveReader<R>
where
    R: Read,
{
    /// Opens an archive for reading.
    pub fn new(mut reader: R) -> Result<Self, Error> {
        let width = usize::from_le_bytes(read_bytes(&mut reader)?);
        let height = usize::from_le_bytes(read_bytes(&mut reader)?);
        let palette_color_count = read_bytes::<_, 1>(&mut reader)?[0];
        Ok(Self {
            reader,
            dimensions: Dimensions {
                width,
                height,
                palette_color_count,
            },
        })
    }
}
