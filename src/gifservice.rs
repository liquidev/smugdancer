//! GIF encoding management service.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use axum::{http::StatusCode, Json};
use parking_lot::Mutex;
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
    /// How many GIFs to remove at a time.
    pub cache_purge_max_count: usize,
}

impl GifServiceConfig {
    pub fn create_cache_dirs(&self) -> Result<(), io::Error> {
        std::fs::create_dir_all(&self.work_dir())?;
        std::fs::create_dir_all(&self.gif_dir())?;
        Ok(())
    }

    pub fn open_cache_database(&self) -> Result<rusqlite::Connection, Error> {
        tracing::debug!("opening connection to cache database");
        let database = rusqlite::Connection::open(self.database())?;
        database.execute(
            r#"
                CREATE TABLE IF NOT EXISTS usage_time (
                    file    TEXT NOT NULL UNIQUE,
                    time    INTEGER NOT NULL
                )
            "#,
            (),
        )?;
        Ok(database)
    }

    pub fn database(&self) -> PathBuf {
        self.cache_dir.join("cache.db")
    }

    pub fn work_dir(&self) -> PathBuf {
        self.cache_dir.join("work")
    }

    pub fn gif_dir(&self) -> PathBuf {
        self.cache_dir.join("gif")
    }
}

pub struct GifService {
    config: Arc<GifServiceConfig>,
    render_requests: mpsc::Sender<RenderRequest>,
    database: Arc<Mutex<rusqlite::Connection>>,
}

struct RenderParams<'s> {
    frame_count: usize,
    config: &'s GifServiceConfig,
}

impl GifService {
    const FRAMES_PATH: &'static str = "data/frames";

    pub fn spawn(config: GifServiceConfig, frame_count: usize) -> Result<GifServiceHandle, Error> {
        let (gif_tx, mut gif_rx) = mpsc::channel(32);
        let (render_tx, mut render_rx) = mpsc::channel(32);

        let database = config
            .open_cache_database()
            .expect("cannot open cache database");
        let database = Arc::new(Mutex::new(database));

        let service = Arc::new(GifService {
            config: Arc::new(config),
            render_requests: render_tx,
            database,
        });
        // NOTE: Render requests are handled in a queue, unlike GIF requests which are handled
        // concurrently.
        tokio::spawn({
            let config = Arc::clone(&service.config);
            async move {
                tracing::info!("render task is ready");
                while let Some(request) = render_rx.recv().await {
                    Self::handle_render_request(
                        request,
                        RenderParams {
                            frame_count,
                            config: &config,
                        },
                    )
                    .await;
                }
            }
        });
        tokio::spawn(async move {
            tracing::info!("GIF task is ready");
            while let Some(request) = gif_rx.recv().await {
                let service = Arc::clone(&service);
                tokio::spawn(async move { service.handle_gif_request(request).await });
            }
        });

        Ok(GifServiceHandle { requests: gif_tx })
    }

    async fn handle_gif_request(&self, request: GifRequest) {
        let GifRequest { speed, responder } = request;
        let _ = responder.send(self.handle_gif_request_inner(speed).await);
    }

    fn get_cached_filename(speed: f64) -> String {
        let bits = speed.to_bits();
        format!("{bits:x}.gif")
    }

    async fn handle_gif_request_inner(&self, speed: f64) -> Result<Vec<u8>, Error> {
        tracing::debug!("handling request for {speed}x speed");
        let cached_filename = self.config.gif_dir().join(Self::get_cached_filename(speed));

        if !cached_filename.exists() {
            // GC errors are non-fatal.
            if let Err(error) = self.collect_garbage().await {
                tracing::error!("{error}")
            }

            tracing::debug!("this speed is not cached yet, rendering");
            let (tx, rx) = oneshot::channel();
            self.render_requests
                .send(RenderRequest {
                    speed,
                    responder: tx,
                })
                .await
                .map_err(|_| Error::GifServiceOffline)?;
            let gif_file = rx.await.map_err(|_| Error::EncodingJobExited)??;
            tokio::fs::rename(&gif_file, &cached_filename)
                .await
                .map_err(Error::CannotRenameGif)?;
        }

        // NOTE: Result is ignored because the task shouldn't panic.
        // If it does, the panic will be logged.
        let _ = tokio::task::spawn_blocking({
            let database = Arc::clone(&self.database);

            let file = cached_filename.clone();
            let file = file.to_str().ok_or(Error::InvalidUtf8)?.to_owned();
            let time = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_err(|_| Error::ClockWentBackwards)?
                .as_secs();

            move || {
                let database = database.lock();
                let mut stmt = database
                    .prepare_cached(
                        r#"
                            INSERT OR REPLACE
                            INTO usage_time (file, time)
                            VALUES (?1, ?2)
                        "#,
                    )
                    .expect("cannot prepare SQL statement");
                stmt.execute((file, time))
            }
        })
        .await;

        let file = tokio::fs::read(&cached_filename)
            .await
            .map_err(Error::CannotReadGif)?;
        Ok(file)
    }

    async fn handle_render_request(request: RenderRequest, params: RenderParams<'_>) {
        let result = Self::render_speed(request.speed, params.frame_count, params.config).await;
        let _ = request.responder.send(result);
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
            // .arg("--height")
            // .arg("360")
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

    async fn collect_garbage(&self) -> Result<(), Error> {
        let mut entries = vec![];
        let mut read_dir = tokio::fs::read_dir(&self.config.gif_dir())
            .await
            .map_err(Error::CollectGarbage)?;
        while let Some(entry) = read_dir.next_entry().await.map_err(Error::CollectGarbage)? {
            let metadata = entry.metadata().await.map_err(Error::CollectGarbage)?;
            entries.push((entry, metadata));
        }

        let mut total_size: u64 = entries.iter().map(|(_, metadata)| metadata.len()).sum();
        if total_size >= self.config.cache_limit {
            tracing::info!(
                "purging cache (exceeded limit of {} bytes - now at {total_size})",
                self.config.cache_limit
            );

            let database = Arc::clone(&self.database);
            let max_count = self.config.cache_purge_max_count;
            let oldest_files: Vec<String> = tokio::task::spawn_blocking(move || {
                let database = database.lock();
                let mut stmt = database
                    .prepare_cached(
                        r#"
                            SELECT file FROM usage_time
                            ORDER BY time ASC
                            LIMIT ?1
                        "#,
                    )
                    .expect("cannot prepare query");
                stmt.query_map((max_count,), |row| row.get(0))
                    .expect("cannot query rows")
                    .filter_map(|r| r.ok())
                    .collect()
            })
            .await
            .map_err(|e| Error::DbQuery(e.to_string()))?;

            let mut to_remove = vec![];
            for filename in oldest_files {
                let path = Path::new(&filename);
                if let Ok(metadata) = path.metadata() {
                    to_remove.push(filename);
                    total_size -= metadata.len();
                    if total_size <= self.config.cache_purge_limit {
                        break;
                    }
                }
            }
            let mut removed = vec![];
            for filename in to_remove {
                match tokio::fs::remove_file(&filename)
                    .await
                    .map_err(Error::CollectGarbage)
                {
                    Ok(_) => {
                        tracing::debug!("cache purge: removed {filename:?}");
                        removed.push(filename);
                    }
                    Err(error) => {
                        tracing::debug!("cache purge: cannot remove {filename:?}: {error}")
                    }
                }
            }
            let database = Arc::clone(&self.database);
            tokio::task::spawn_blocking(move || {
                let database = database.lock();
                let mut stmt = database
                    .prepare_cached(
                        r#"
                            DELETE FROM usage_time
                            WHERE file = ?1
                        "#,
                    )
                    .expect("cannot prepare deletion query");
                for filename in removed {
                    // NOTE: Should always succeed so we ignore the result.
                    let _ = stmt.execute((filename,));
                }
            });
        }

        Ok(())
    }
}

struct GifRequest {
    speed: f64,
    responder: oneshot::Sender<Result<Vec<u8>, Error>>,
}

struct RenderRequest {
    speed: f64,
    responder: oneshot::Sender<Result<PathBuf, Error>>,
}

#[derive(Clone)]
pub struct GifServiceHandle {
    requests: mpsc::Sender<GifRequest>,
}

impl GifServiceHandle {
    pub async fn request_speed(&self, speed: f64) -> Result<Vec<u8>, Error> {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(GifRequest {
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

    #[error("GIF encoding process: {0}")]
    Encoder(io::Error),
    #[error("Cache database: {0}")]
    CacheDb(#[from] rusqlite::Error),
    #[error("Database query: {0}")]
    DbQuery(String),
    #[error("Cannot read rendered GIF: {0}")]
    CannotReadGif(io::Error),
    #[error("Cannot rename rendered GIF: {0}")]
    CannotRenameGif(io::Error),
    #[error("Cannot send request to GIF service because it is offline (did the thread panic?)")]
    GifServiceOffline,
    #[error("Internal encoding job failure (did not receive rendered GIF)")]
    EncodingJobExited,
    #[error("Invalid UTF-8")]
    InvalidUtf8,
    #[error("System clock went backwards")]
    ClockWentBackwards,

    #[error("Cache garbage collection I/O: {0}")]
    CollectGarbage(io::Error),
}

impl Error {
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::SpeedTooFast | Self::SpeedTooSlow => StatusCode::BAD_REQUEST,
            Self::Encoder(_)
            | Self::CacheDb(_)
            | Self::DbQuery(_)
            | Self::CannotReadGif(_)
            | Self::CannotRenameGif(_)
            | Self::GifServiceOffline
            | Self::EncodingJobExited
            | Self::InvalidUtf8
            | Self::ClockWentBackwards
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
