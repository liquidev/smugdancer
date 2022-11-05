use std::{fmt::Display, io, sync::Arc};

use axum::{http::StatusCode, Json};
use serde::Serialize;
use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum Error {
    #[error("Hat Kid got incarcerated for speeding on a highway.")]
    SpeedTooFast,
    #[error("yawn…")]
    SpeedTooSlow,

    #[error("GIF encoding process: {0}")]
    Encoder(io::Error),
    #[error("GIF encoder finished with a non-zero exit code")]
    EncoderExitCode,
    #[error("Cache database: {0}")]
    CacheDb(#[from] rusqlite::Error),
    #[error("Database query: {0}")]
    DbQuery(String),
    #[error("Cannot read rendered GIF: {0}")]
    CannotReadGif(io::Error),
    #[error("Cannot write rendered GIF: {0}")]
    CannotWriteGif(io::Error),
    #[error("Cannot send request to GIF service because it is offline (did the thread panic?)")]
    GifServiceOffline,
    #[error("Internal encoding job failure (did not receive rendered GIF)")]
    EncodingJobExited,
    #[error("Invalid UTF-8")]
    InvalidUtf8,
    #[error("System clock went backwards")]
    ClockWentBackwards,
    #[error("Directory cannot be set up: {0}")]
    DirSetup(io::Error),
    #[error("Render failed: {0}")]
    RenderFailed(Arc<Error>),

    #[error("Cache garbage collection I/O: {0}")]
    CollectGarbage(io::Error),
}

impl Error {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::SpeedTooFast | Self::SpeedTooSlow => StatusCode::BAD_REQUEST,
            Self::Encoder(_)
            | Self::EncoderExitCode
            | Self::CacheDb(_)
            | Self::DbQuery(_)
            | Self::CannotReadGif(_)
            | Self::CannotWriteGif(_)
            | Self::GifServiceOffline
            | Self::EncodingJobExited
            | Self::InvalidUtf8
            | Self::ClockWentBackwards
            | Self::DirSetup(_)
            | Self::CollectGarbage(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::RenderFailed(error) => error.status_code(),
        }
    }

    pub fn to_response(&self) -> ErrorResponse {
        (
            self.status_code(),
            Json(ErrorMessage {
                error: match self {
                    Self::RenderFailed(error) if error.status_code() == StatusCode::BAD_REQUEST => {
                        error.to_string()
                    }
                    _ => self.to_string(),
                },
            }),
        )
    }
}
