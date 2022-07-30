//! Algorithm to extract a palette from an image.

use image::{Rgb, RgbImage};

fn calculate_color_range(image: &[[u8; 3]]) -> [u8; 3] {
    let (mut min_r, mut min_g, mut min_b) = (255, 255, 255);
    let (mut max_r, mut max_g, mut max_b) = (0, 0, 0);

    for pixel in image {
        (min_r, max_r) = (min_r.min(pixel[0]), max_r.max(pixel[0]));
        (min_g, max_g) = (min_g.min(pixel[1]), max_g.max(pixel[1]));
        (min_b, max_b) = (min_b.min(pixel[2]), max_b.max(pixel[2]));
    }

    [max_r - min_r, max_g - min_g, max_b - min_b]
}

fn max_channel(pixel: [u8; 3]) -> usize {
    pixel
        .into_iter()
        .position(|x| x == *pixel.iter().max().unwrap())
        .unwrap()
}

fn halves(pixels: &mut [[u8; 3]]) -> (&mut [[u8; 3]], &mut [[u8; 3]]) {
    pixels.split_at_mut(pixels.len() / 2)
}

/// Extracts an RGB palette from the image with `2.pow(subdiv)` colors.
pub fn extract_palette(image: &RgbImage, subdiv: usize) -> Vec<[u8; 3]> {
    let mut pixels: Vec<_> = image.pixels().map(|&Rgb(color)| color).collect();
    pixels.sort();
    pixels.dedup();
    println!("{}", pixels.len());
    let mut palette = vec![];

    fn subdivide_bucket(
        palette: &mut Vec<[u8; 3]>,
        pixels: &mut [[u8; 3]],
        current_subdiv: usize,
        max_subdiv: usize,
    ) {
        if current_subdiv == max_subdiv {
            let sum_r: usize = pixels.iter().map(|&[r, _, _]| r as usize).sum();
            let sum_g: usize = pixels.iter().map(|&[_, g, _]| g as usize).sum();
            let sum_b: usize = pixels.iter().map(|&[_, _, b]| b as usize).sum();
            let color = [
                (sum_r / pixels.len()) as u8,
                (sum_g / pixels.len()) as u8,
                (sum_b / pixels.len()) as u8,
            ];
            palette.push(color);
        } else {
            let channel = max_channel(calculate_color_range(pixels));
            pixels.sort_by(|a, b| a[channel].cmp(&b[channel]));
            let (fst, snd) = halves(pixels);
            subdivide_bucket(palette, fst, current_subdiv + 1, max_subdiv);
            subdivide_bucket(palette, snd, current_subdiv + 1, max_subdiv);
        }
    }

    subdivide_bucket(&mut palette, &mut pixels, 0, subdiv);

    palette
}

fn color_difference(a: [u8; 3], b: [u8; 3]) -> u16 {
    let dr = a[0].abs_diff(b[0]) as u16;
    let dg = a[1].abs_diff(b[1]) as u16;
    let db = a[2].abs_diff(b[2]) as u16;
    dr + dg + db
}

pub fn quantize(image: &RgbImage, palette: &[[u8; 3]]) -> Vec<u8> {
    image
        .pixels()
        .map(|&Rgb(a)| {
            palette
                .iter()
                .enumerate()
                .min_by_key(|(_, &b)| color_difference(a, b))
                .unwrap()
                .0 as u8
        })
        .collect()
}
