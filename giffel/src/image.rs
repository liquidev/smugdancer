//! Image utilities.

use std::ops::Index;

pub struct Image<T> {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<T>,
}

impl<T> Index<(usize, usize)> for Image<T> {
    type Output = T;

    fn index(&self, index: (usize, usize)) -> &Self::Output {
        &self.pixels[index.0 + index.1 * self.width]
    }
}
