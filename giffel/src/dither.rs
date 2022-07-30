use image::{Rgb, RgbImage};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use wide::f32x4;

// https://bisqwit.iki.fi/story/howto/dither/jy/
// This uses the Knoll dithering algorithm whose patent expired in 2019.

const MATRIX_SIZE: usize = 8;
const MATRIX_LEN: usize = MATRIX_SIZE * MATRIX_SIZE;
#[rustfmt::skip]
const MATRIX: [u8; MATRIX_LEN] = [
    0, 48, 12, 60, 3, 51, 15, 63,
    32, 16, 44, 28, 35, 19, 47, 31,
    8, 56, 4, 52, 11, 59, 7, 55,
    40, 24, 36, 20, 43, 27, 39, 23,
    2, 50, 14, 62, 1, 49, 13, 61,
    34, 18, 46, 30, 33, 17, 45, 29,
    10, 58, 6, 54, 9, 57, 5, 53,
    42, 26, 38, 22, 41, 25, 37, 21,
];

fn luma(color: [u8; 3]) -> f32 {
    let color = color.map(|x| x as f32);
    (color[0] * 0.299 + color[1] * 0.587 + color[2] * 0.114) / 255.0
}

pub fn compare_colors(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (a, b) = (
        f32x4::new([a[0] as f32, a[1] as f32, a[2] as f32, 0.0]),
        f32x4::new([b[0] as f32, b[1] as f32, b[2] as f32, 0.0]),
    );

    const DIV_255: f32 = 1.0 / 255.0;

    let coeffs = f32x4::new([0.299, 0.587, 0.114, 0.0]);
    let luma1 = (a * coeffs).reduce_add();
    let luma2 = (b * coeffs).reduce_add();
    let luma_diff = (luma2 - luma1) * DIV_255;
    let diffs = (a - b) * DIV_255;

    (diffs * diffs * coeffs).reduce_add() * 0.75 + luma_diff * luma_diff
}

type MixingPlan = [usize; MATRIX_LEN];

fn devise_best_mixing_plan(
    color: [u8; 3],
    palette: &[[u8; 3]],
    palette_luma: &[f32],
    threshold: f32,
) -> MixingPlan {
    let src = color.map(|x| x as u32);
    let mut result = [0; MATRIX_LEN];

    let mut e = [0, 0, 0];
    for (c, out_color) in result.iter_mut().enumerate() {
        let t = [
            src[0] + (e[0] as f32 * threshold) as u32,
            src[1] + (e[1] as f32 * threshold) as u32,
            src[2] + (e[2] as f32 * threshold) as u32,
        ]
        .map(|x| x.clamp(0, 255) as u8);

        let mut least_penalty = f32::INFINITY;
        let mut chosen = c % 16;
        for (index, &palette_color) in palette.iter().enumerate() {
            let penalty = compare_colors(palette_color, t);
            if penalty < least_penalty {
                least_penalty = penalty;
                chosen = index;
            }
        }
        *out_color = chosen;
        let pc = palette[chosen].map(|x| x as i32);
        e[0] += src[0] as i32 - pc[0];
        e[1] += src[1] as i32 - pc[1];
        e[2] += src[2] as i32 - pc[2];
    }

    result.sort_by(|&a, &b| palette_luma[a].total_cmp(&palette_luma[b]));

    result
}

pub fn dither(image: &RgbImage, palette: &[[u8; 3]], threshold: f32) -> Vec<u8> {
    let palette_luma: Vec<_> = palette.iter().map(|&x| luma(x)).collect();

    let pixel_count = image.width() as usize * image.height() as usize;

    (0..pixel_count)
        .into_par_iter()
        .map(|pixel_index| {
            let x = pixel_index % image.width() as usize;
            let y = pixel_index / image.width() as usize;
            let Rgb(pixel) = image[(x as u32, y as u32)];
            let matrix_value = MATRIX[(x & 7) + ((y & 7) << 3)];
            let plan = devise_best_mixing_plan(pixel, palette, &palette_luma, threshold);
            let index = plan[matrix_value as usize];
            index as u8
        })
        .collect()
}
