//! Image utilities.

use std::ops::{Index, IndexMut};

#[derive(Clone)]
pub struct Image<T> {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<T>,
}

impl<T> Image<T> {
    pub fn pixel_index(&self, position: (usize, usize)) -> usize {
        position.0 + position.1 * self.width
    }
}

impl<T> Index<(usize, usize)> for Image<T> {
    type Output = T;

    fn index(&self, index: (usize, usize)) -> &Self::Output {
        &self.pixels[self.pixel_index(index)]
    }
}

impl<T> IndexMut<(usize, usize)> for Image<T> {
    fn index_mut(&mut self, index: (usize, usize)) -> &mut Self::Output {
        let index = self.pixel_index(index);
        &mut self.pixels[index]
    }
}
