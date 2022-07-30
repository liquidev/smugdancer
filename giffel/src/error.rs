use image::ImageError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Error while loading image: {0}")]
    Image(#[from] ImageError),
}
