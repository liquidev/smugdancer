//! GIF encoding management service.

use std::{
    io,
    path::{Path, PathBuf},
};

use axum::{http::StatusCode, Json};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    process::Command,
    sync::{mpsc, oneshot},
};

use crate::common::{generate_unique_filename, ErrorMessage, ErrorResponse};

#[derive(Clone, Deserialize)]
pub struct GifServiceConfig {
    /// The gifski executable.
    pub gifski: PathBuf,

    /// The cache directory.
    pub cache_dir: PathBuf,
    /// When to start purging the cache (in bytes taken up by GIFs.)
    pub cache_limit: u64,
    /// When to stop removing old GIFs.
    pub cache_purge_limit: u64,
}

impl GifServiceConfig {
    pub fn create_cache_dirs(&self) -> Result<(), io::Error> {
        std::fs::create_dir_all(&self.work_dir())?;
        std::fs::create_dir_all(&self.gif_dir())?;
        Ok(())
    }

    pub fn work_dir(&self) -> PathBuf {
        self.cache_dir.join("work")
    }

    pub fn gif_dir(&self) -> PathBuf {
        self.cache_dir.join("gif")
    }
}

pub struct GifService {
    config: GifServiceConfig,
    frame_count: usize,
    requests: mpsc::Receiver<EncodeRequest>,
}

impl GifService {
    const FRAMES_PATH: &'static str = "data/frames";

    pub fn spawn(config: GifServiceConfig, frame_count: usize) -> GifServiceHandle {
        let (req_tx, req_rx) = mpsc::channel(32);

        let mut service = GifService {
            config,
            frame_count,
            requests: req_rx,
        };
        tokio::spawn(async move {
            tracing::info!("GIF service is ready");
            // NOTE: Only one request is handled at a time. We treat the channel as a queue so as
            // not to overload the server by encoding GIFs. Maybe in the future we can improve on
            // this, especially if I can rent a better VPS.
            while let Some(request) = service.requests.recv().await {
                service.handle_request(request).await;
            }
        });

        GifServiceHandle { requests: req_tx }
    }

    async fn handle_request(&mut self, request: EncodeRequest) {
        let EncodeRequest { speed, responder } = request;
        let _ = responder.send(self.handle_request_inner(speed).await);
    }

    fn get_cached_filename(speed: f64) -> String {
        let bits = speed.to_bits();
        format!("{bits:x}.gif")
    }

    async fn handle_request_inner(&mut self, speed: f64) -> Result<Vec<u8>, Error> {
        tracing::debug!("handling request for {speed}x speed");
        let cached_filename = self.config.gif_dir().join(Self::get_cached_filename(speed));

        if !cached_filename.exists() {
            // GC errors are non-fatal.
            if let Err(error) = Self::collect_garbage(&self.config).await {
                tracing::error!("{error}")
            }

            tracing::debug!("this speed is not cached yet, rendering");
            let gif_file = Self::render_speed(speed, self.frame_count, &self.config).await?;
            tokio::fs::rename(&gif_file, &cached_filename)
                .await
                .map_err(Error::CannotRenameGif)?;
        }

        let file = tokio::fs::read(&cached_filename)
            .await
            .map_err(Error::CannotReadGif)?;
        Ok(file)
    }

    async fn render_speed(
        speed: f64,
        frame_count: usize,
        config: &GifServiceConfig,
    ) -> Result<PathBuf, Error> {
        let output_frames = (frame_count as f64 / speed).floor() as usize;
        if output_frames <= 1 {
            tracing::debug!("requested speed is too fast");
            return Err(Error::SpeedTooFast);
        }
        if output_frames > 900 {
            tracing::debug!("requested speed is too slow");
            return Err(Error::SpeedTooSlow);
        }

        let mut accumulator: f64 = 0.0;
        let input_filenames: Vec<_> = (0..output_frames)
            .filter_map(|_| {
                let input_frame = accumulator.floor() as usize + 1;
                accumulator += speed;
                let path = Path::new(Self::FRAMES_PATH).join(format!("{input_frame}.png"));
                path.to_str().map(|x| x.to_owned())
            })
            .collect();

        let output_filename = format!("{}.gif", generate_unique_filename(32));
        let output_filename = config.work_dir().join(&output_filename);
        Command::new(&config.gifski)
            .arg("--fps")
            .arg("50")
            .arg("--height")
            .arg("360")
            .arg("--output")
            .arg(&output_filename)
            .arg("--no-sort")
            .args(&input_filenames)
            .spawn()
            .map_err(Error::Encoder)?
            .wait()
            .await
            .map_err(Error::Encoder)?;

        tracing::debug!("render of {speed}x speed complete");

        Ok(output_filename)
    }

    async fn collect_garbage(config: &GifServiceConfig) -> Result<(), Error> {
        let mut entries = vec![];
        let mut read_dir = tokio::fs::read_dir(&config.gif_dir())
            .await
            .map_err(Error::CollectGarbage)?;
        while let Some(entry) = read_dir.next_entry().await.map_err(Error::CollectGarbage)? {
            let metadata = entry.metadata().await.map_err(Error::CollectGarbage)?;
            entries.push((entry, metadata));
        }

        let mut total_size: u64 = entries.iter().map(|(_, metadata)| metadata.len()).sum();
        if total_size >= config.cache_limit {
            tracing::info!(
                "purging cache (exceeded limit of {} bytes - now at {total_size})",
                config.cache_limit
            );

            entries.sort_by(|(_, a), (_, b)| {
                a.created()
                    .expect("you're kidding, right?")
                    .cmp(&b.created().expect("we've been tricked"))
            });

            let mut to_remove = vec![];
            for (entry, metadata) in &entries {
                to_remove.push(entry);
                total_size -= metadata.len();
                if total_size <= config.cache_purge_limit {
                    break;
                }
            }
            for entry in to_remove {
                let path = entry.path();
                tokio::fs::remove_file(&path)
                    .await
                    .map_err(Error::CollectGarbage)?;
                tracing::debug!("cache purge: removed {path:?}");
            }
        }

        Ok(())
    }
}

struct EncodeRequest {
    speed: f64,
    responder: oneshot::Sender<Result<Vec<u8>, Error>>,
}

#[derive(Clone)]
pub struct GifServiceHandle {
    requests: mpsc::Sender<EncodeRequest>,
}

impl GifServiceHandle {
    pub async fn request_speed(&self, speed: f64) -> Result<Vec<u8>, Error> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(EncodeRequest {
                speed,
                responder: tx,
            })
            .await
            .map_err(|_| Error::GifServiceOffline)?;
        match rx.await {
            Ok(r) => r,
            Err(_) => Err(Error::EncodingJobExited),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Hat Kid got incarcerated for speeding on a highway.")]
    SpeedTooFast,
    #[error("yawn…")]
    SpeedTooSlow,

    #[error("Error while handling GIF encoding process: {0}")]
    Encoder(io::Error),
    #[error("Cannot read rendered GIF: {0}")]
    CannotReadGif(io::Error),
    #[error("Cannot rename rendered GIF: {0}")]
    CannotRenameGif(io::Error),
    #[error("Cannot send request to GIF service because it is offline (did the thread panic?)")]
    GifServiceOffline,
    #[error("Internal encoding job failure (did not receive rendered GIF)")]
    EncodingJobExited,

    #[error("Cache garbage collection I/O error: {0}")]
    CollectGarbage(io::Error),
}

impl Error {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::SpeedTooFast | Self::SpeedTooSlow => StatusCode::BAD_REQUEST,
            Self::Encoder(_)
            | Self::CannotReadGif(_)
            | Self::CannotRenameGif(_)
            | Self::GifServiceOffline
            | Self::EncodingJobExited
            | Self::CollectGarbage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn to_response(&self) -> ErrorResponse {
        (
            self.status_code(),
            Json(ErrorMessage {
                error: self.to_string(),
            }),
        )
    }
}
