//! Render cache management service.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error, info, info_span};

use crate::{common::Error, render_service::RenderServiceHandle};

#[derive(Clone, Deserialize)]
pub struct CacheServiceConfig {
    /// The cache directory.
    pub cache_dir: PathBuf,
    /// The path to the cache database.
    pub database: PathBuf,
    /// When to start purging the cache (in bytes taken up by GIFs.)
    pub limit: u64,
    /// When to stop removing old GIFs.
    pub purge_limit: u64,
    /// How many GIFs to remove at a time.
    pub purge_max_count: usize,
}

impl CacheServiceConfig {
    pub fn setup(&self) -> Result<rusqlite::Connection, Error> {
        debug!("creating cache directories");
        std::fs::create_dir_all(&self.cache_dir).map_err(Error::DirSetup)?;

        debug!("opening connection to cache database");
        let database = rusqlite::Connection::open(&self.database)?;
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
}

pub struct GifService {
    config: CacheServiceConfig,
    render_service: RenderServiceHandle,
    database: Arc<Mutex<rusqlite::Connection>>,
}

impl GifService {
    pub fn spawn(
        config: CacheServiceConfig,
        render_service: RenderServiceHandle,
    ) -> Result<CacheServiceHandle, Error> {
        let (requests_tx, mut requests_rx) = mpsc::channel(32);

        let database = config.setup()?;
        let database = Arc::new(Mutex::new(database));

        let service = Arc::new(GifService {
            config,
            render_service,
            database,
        });
        tokio::spawn(async move {
            info!("cache task is ready");
            while let Some(request) = requests_rx.recv().await {
                let service = Arc::clone(&service);
                tokio::spawn(async move { service.handle_request(request).await });
            }
        });

        Ok(CacheServiceHandle {
            requests: requests_tx,
        })
    }

    async fn handle_request(&self, request: GifRequest) {
        let GifRequest { speed, responder } = request;
        let _ = responder.send(self.handle_request_inner(speed).await);
    }

    async fn handle_request_inner(&self, speed: f64) -> Result<Vec<u8>, Error> {
        debug!(speed, "handling cache request");
        let cached_filename = self.config.cache_dir.join(Self::get_cached_filename(speed));

        let file = if !cached_filename.exists() {
            // GC errors are non-fatal.
            if let Err(error) = self.collect_garbage().await {
                error!("{error}")
            }

            debug!("this speed is not cached yet, rendering");
            let (gif, position_in_queue) = self
                .render_service
                .render_speed(speed)
                .await
                .map_err(Error::RenderFailed)?;
            if position_in_queue == 0 {
                tokio::fs::write(&cached_filename, &gif)
                    .await
                    .map_err(Error::CannotWriteGif)?;
            }

            gif
        } else {
            tokio::fs::read(&cached_filename)
                .await
                .map_err(Error::CannotReadGif)?
        };

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

        Ok(file)
    }

    fn get_cached_filename(speed: f64) -> String {
        let bits = speed.to_bits();
        format!("{bits:x}.gif")
    }

    async fn collect_garbage(&self) -> Result<(), Error> {
        let mut entries = vec![];
        let mut read_dir = tokio::fs::read_dir(&self.config.cache_dir)
            .await
            .map_err(Error::CollectGarbage)?;
        while let Some(entry) = read_dir.next_entry().await.map_err(Error::CollectGarbage)? {
            let metadata = entry.metadata().await.map_err(Error::CollectGarbage)?;
            entries.push((entry, metadata));
        }

        let mut total_size: u64 = entries.iter().map(|(_, metadata)| metadata.len()).sum();
        if total_size >= self.config.limit {
            let _span = info_span!("cache_purge");
            info!(
                self.config.limit,
                total_size, "purging cache (limit was exceeded)"
            );

            let database = Arc::clone(&self.database);
            let max_count = self.config.purge_max_count;
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
                    if total_size <= self.config.purge_limit {
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
                        debug!(?filename, "removed file");
                        removed.push(filename);
                    }
                    Err(error) => {
                        debug!(?filename, %error, "cannot remove file")
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

#[derive(Clone)]
pub struct CacheServiceHandle {
    requests: mpsc::Sender<GifRequest>,
}

impl CacheServiceHandle {
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
