//! Alpha cropping. Crops bitmaps to the non-alpha containing region to save space.

use rayon::prelude::*;

use crate::image::Image;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

pub fn find_opaque_frame(image: &Image<u8>) -> Rect {
    let (left, right) = (0..image.height)
        .into_par_iter()
        .map(|y| {
            let left = (0..image.width).find(|&x| image[(x, y)] != 255);
            let right = (0..image.width).rfind(|&x| image[(x, y)] != 255);
            (left.unwrap_or(image.width), right.unwrap_or(0))
        })
        .reduce(
            || (image.width, 0),
            |(min_accum, max_accum), (min, max)| (min_accum.min(min), max_accum.max(max)),
        );

    let (top, bottom) = (0..image.width)
        .into_par_iter()
        .map(|x| {
            let top = (0..image.height).find(|&y| image[(x, y)] != 255);
            let bottom = (0..image.height).rfind(|&y| image[(x, y)] != 255);
            (top.unwrap_or(image.height), bottom.unwrap_or(0))
        })
        .reduce(
            || (image.height, 0),
            |(min_accum, max_accum), (min, max)| (min_accum.min(min), max_accum.max(max)),
        );

    Rect {
        x: left,
        y: top,
        width: right - left + 1,
        height: bottom - top + 1,
    }
}

pub fn crop(image: &Image<u8>, rect: &Rect) -> Image<u8> {
    let pixels = vec![0; rect.width * rect.height];
    let mut result = Image {
        width: rect.width,
        height: rect.height,
        pixels,
    };

    for y in 0..rect.height {
        let src_index = image.pixel_index((rect.x, rect.y + y));
        let dst_index = result.pixel_index((0, y));
        let scanline = &image.pixels[src_index..src_index + rect.width];
        result.pixels[dst_index..dst_index + rect.width].copy_from_slice(scanline);
    }

    result
}
