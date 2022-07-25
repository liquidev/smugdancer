use std::fmt::Display;

use axum::{http::StatusCode, Json};
use nanorand::Rng;
use serde::Serialize;

#[derive(Serialize)]
pub struct ErrorMessage {
    pub error: String,
}

pub type ErrorResponse = (StatusCode, Json<ErrorMessage>);

pub fn error_response<E>(status_code: StatusCode, error: E) -> ErrorResponse
where
    E: Display,
{
    (
        status_code,
        Json(ErrorMessage {
            error: error.to_string(),
        }),
    )
}

pub fn generate_unique_filename(len: usize) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    let mut result = String::new();
    let mut rng = nanorand::tls_rng();
    for _ in 0..len {
        result.push(char::from(CHARSET[rng.generate_range(0..CHARSET.len())]));
    }
    result
}