//! Support for giffel archive files.

use std::{
    io::{Read, Seek, SeekFrom, Write},
    mem::size_of,
};

use crate::{error::Error, image::Image};

pub const MAGIC: &[u8] = b"GIFFEL22";
pub const HEADER_SIZE: usize = MAGIC.len() + size_of::<u16>() * 2 + size_of::<u8>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u16,
    pub height: u16,
    pub palette_color_count: u8,
}

impl Dimensions {
    fn of(image: &Image<u8>, palette: &[[u8; 3]]) -> Result<Self, Error> {
        if palette.is_empty() {
            return Err(Error::PaletteIsEmpty);
        }
        Ok(Self {
            width: u16::try_from(image.width).map_err(|_| Error::FramesTooBig)?,
            height: u16::try_from(image.height).map_err(|_| Error::FramesTooBig)?,
            palette_color_count: u8::try_from(palette.len() - 1)
                .map_err(|_| Error::PaletteTooBig)?,
        })
    }

    /// Returns the width as a `usize`.
    pub fn width(&self) -> usize {
        self.width as usize
    }

    /// Returns the height as a `usize`.
    pub fn height(&self) -> usize {
        self.height as usize
    }

    /// Returns the number of colors in the palette. This is different from the field, which
    /// is biased by -1 (a value of 0 means 1 color.)
    pub fn palette_color_count(&self) -> usize {
        self.palette_color_count as usize + 1
    }

    /// Returns the size (in bytes) of a single frame saved in a giffel archive with these
    /// dimensions.
    fn frame_size(&self) -> usize {
        self.width() * self.height() + (self.palette_color_count()) * 3
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
        self.writer.write_all(MAGIC)?;
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
    pub frame_count: usize,
}

impl<R> ArchiveReader<R>
where
    R: Read + Seek,
{
    /// Opens an archive for reading.
    pub fn new(mut reader: R) -> Result<Self, Error> {
        let magic = read_bytes::<_, { MAGIC.len() }>(&mut reader)?;
        if magic != MAGIC {
            return Err(Error::InvalidMagic);
        }

        let width = u16::from_le_bytes(read_bytes(&mut reader)?);
        let height = u16::from_le_bytes(read_bytes(&mut reader)?);
        let palette_color_count = read_bytes::<_, 1>(&mut reader)?[0];
        let dimensions = Dimensions {
            width,
            height,
            palette_color_count,
        };

        let archive_size = reader.seek(SeekFrom::End(0))? as usize;
        let frame_count = (archive_size - HEADER_SIZE) / dimensions.frame_size();

        Ok(Self {
            reader,
            dimensions,
            frame_count,
        })
    }

    /// Read the frame at the specified index. Returns an error if there's no frame with the given
    /// index. Indices start at 1.
    pub fn read_frame(&mut self, index: usize) -> Result<(Image<u8>, Vec<[u8; 3]>), Error> {
        if index == 0 || index > self.frame_count {
            return Err(Error::FrameOutOfBounds {
                got: index,
                count: self.frame_count,
            });
        }
        let index = index - 1;
        let offset = HEADER_SIZE + index * self.dimensions.frame_size();
        self.reader.seek(SeekFrom::Start(offset as u64))?;

        let mut palette = vec![0; self.dimensions.palette_color_count() * 3];
        self.reader.read_exact(&mut palette)?;
        let mut pixels = vec![0; self.dimensions.width() * self.dimensions.height()];
        self.reader.read_exact(&mut pixels)?;

        Ok((
            Image {
                width: self.dimensions.width(),
                height: self.dimensions.height(),
                pixels,
            },
            palette
                .chunks_exact(3)
                .map(|a| [a[0], a[1], a[2]])
                .collect(),
        ))
    }
}
