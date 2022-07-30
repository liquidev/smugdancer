use image::{Rgb, RgbImage};

fn color_difference(a: [u8; 3], b: [u8; 3]) -> u16 {
    let dr = a[0].abs_diff(b[0]) as u16;
    let dg = a[1].abs_diff(b[1]) as u16;
    let db = a[2].abs_diff(b[2]) as u16;
    dr + dg + db
}

pub fn dont_dither(image: &RgbImage, palette: &[[u8; 3]]) -> Vec<u8> {
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
