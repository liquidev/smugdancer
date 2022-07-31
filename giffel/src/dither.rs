use crate::{colorspace::Oklab, image::Image};

// https://bisqwit.iki.fi/story/howto/dither/jy/
// This uses the Knoll dithering algorithm whose patent expired in 2019.
// The code has been altered to use the Oklab color space, which makes compare_colors
// *a lot* faster.

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

pub fn compare_colors(a: Oklab, b: Oklab) -> f32 {
    let dl = b.l - a.l;
    let da = b.a - a.a;
    let db = b.b - a.b;
    dl * dl * 2.0 + da * da + db * db
}

type MixingPlan = [usize; MATRIX_LEN];

fn devise_best_mixing_plan(color: Oklab, palette: &[Oklab], threshold: f32) -> MixingPlan {
    let mut result = [0; MATRIX_LEN];

    let mut e = Oklab {
        l: 0.0,
        a: 0.0,
        b: 0.0,
    };
    for (c, out_color) in result.iter_mut().enumerate() {
        let t = Oklab {
            l: color.l + (e.l * threshold),
            a: color.a + (e.a * threshold),
            b: color.b + (e.b * threshold),
        };
        let mut least_penalty = f32::INFINITY;
        let mut chosen = c % palette.len();
        for (index, &palette_color) in palette.iter().enumerate() {
            let penalty = compare_colors(palette_color, t);
            if penalty < least_penalty {
                least_penalty = penalty;
                chosen = index;
            }
        }
        *out_color = chosen;
        let pc = palette[chosen];
        e.l += color.l - pc.l;
        e.a += color.a - pc.a;
        e.b += color.b - pc.b;
    }

    result.sort_by(|&a, &b| palette[a].l.total_cmp(&palette[b].l));

    result
}

pub fn dither(image: &Image<Oklab>, palette: &[Oklab], threshold: f32) -> Image<u8> {
    let pixel_count = image.width * image.height;

    Image {
        width: image.width,
        height: image.height,
        pixels: (0..pixel_count)
            .into_iter()
            .map(|pixel_index| {
                let x = pixel_index % image.width;
                let y = pixel_index / image.width;
                let pixel = image[(x, y)];
                let matrix_value = MATRIX[(x & 7) + ((y & 7) << 3)];
                let plan = devise_best_mixing_plan(pixel, palette, threshold);
                let index = plan[matrix_value as usize];
                index as u8
            })
            .collect(),
    }
}
