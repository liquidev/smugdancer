use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::{net::SocketAddr, sync::Arc};

use axum::http::StatusCode;
use axum::Json;
use axum::{
    extract::Path as UrlPath,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use handlebars::Handlebars;
use nanorand::Rng;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const CONFIG_PATH: &str = "smugdancer.toml";
const FRAMES_PATH: &str = "data/frames";

// NOTE: 50 fps is a GIF limitation. See index.hbs.
const ANIMATION_FPS: f64 = 50.0;
// This is the number of times Hat Kid waves her hands back and forth in the animation.
const WAVE_COUNT: f64 = 12.0;

#[derive(Deserialize)]
struct Config {
    /// The port under which smugdancer should serve.
    port: u16,
    /// The root URL that's shown on the documentation website.
    root: String,
    /// The path to the `gifski` executable.
    gifski: PathBuf,

    cache: CacheConfig,
}

#[derive(Deserialize)]
struct CacheConfig {
    /// The cache directory.
    dir: PathBuf,
    /// When to start purging the cache (in bytes taken up by GIFs.)
    limit: u64,
    /// When to stop removing old GIFs.
    purge_limit: u64,
}

fn count_frames() -> usize {
    std::fs::read_dir(FRAMES_PATH)
        .expect("cannot read_dir from FRAMES_PATH")
        .filter(|result| result.is_ok())
        .count()
}

fn get_minimum_bpm(frame_count: f64) -> f64 {
    WAVE_COUNT * ANIMATION_FPS * 60.0 / frame_count
}

fn quantize_bpm_to_nearest_supported(bpm: f64) -> f64 {
    let unrounded_frame_count = WAVE_COUNT * ANIMATION_FPS * 60.0 / bpm;
    let frame_count = unrounded_frame_count.floor();
    WAVE_COUNT * ANIMATION_FPS * 60.0 / frame_count
}

#[derive(Serialize)]
struct IndexData {
    root: String,
    minimum_bpm: f64,
}

fn render_index(index_data: IndexData) -> String {
    const INDEX_HBS: &str = include_str!("index.hbs");
    let mut hbs = Handlebars::new();
    hbs.register_template_string("index_hbs", INDEX_HBS)
        .expect("error in index.hbs template");

    hbs.render("index_hbs", &index_data)
        .expect("cannot render index template")
}

fn generate_unique_filename(len: usize) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    let mut result = String::new();
    let mut rng = nanorand::tls_rng();
    for _ in 0..len {
        result.push(char::from(CHARSET[rng.generate_range(0..CHARSET.len())]));
    }
    result
}

struct State {
    /// The index containing documentation.
    index: String,
    /// The number of frames. Note that the frames' filenames must start at 1 and end at `frame_count`.
    frame_count: usize,
    /// The source animation's tempo.
    source_bpm: f64,
    /// The path to `gifski`.
    gifski: PathBuf,
    /// The cache directory that contains GIF files still being processed by gifski.
    cache_work_dir: PathBuf,
    /// The cache directory that contains finished GIF files.
    cache_gif_dir: PathBuf,
    cache_config: CacheConfig,
}

#[derive(Serialize)]
struct ErrorMessage {
    message: String,
}

type ErrorResponse = (StatusCode, Json<ErrorMessage>);

fn error<E>(status: StatusCode, err: E) -> ErrorResponse
where
    E: Display,
{
    (
        status,
        Json(ErrorMessage {
            message: err.to_string(),
        }),
    )
}

async fn index(state: Arc<State>) -> Html<String> {
    Html(state.index.clone())
}

async fn flush_cache(config: &CacheConfig, dir: &Path) -> anyhow::Result<()> {
    // This is probably racy af.
    let mut entries = vec![];
    let mut read_dir = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = read_dir.next_entry().await? {
        let metadata = entry.metadata().await?;
        entries.push((entry, metadata));
    }

    let mut total_size: u64 = entries.iter().map(|(_, metadata)| metadata.len()).sum();
    if total_size >= config.limit {
        tracing::info!(
            "purging cache (exceeded limit of {} bytes - now at {total_size})",
            config.limit
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
            if total_size <= config.purge_limit {
                break;
            }
        }
        for entry in to_remove {
            let path = entry.path();
            tokio::fs::remove_file(&path).await?;
            tracing::debug!("cache purge: removed {path:?}");
        }
    }

    Ok(())
}

async fn render_animation(
    state: Arc<State>,
    UrlPath(unquantized_bpm): UrlPath<f64>,
) -> Result<Response, ErrorResponse> {
    let bpm = quantize_bpm_to_nearest_supported(unquantized_bpm);
    tracing::debug!("serving {bpm} bpm (quantized from {unquantized_bpm} bpm)");
    let cached_filename = state.cache_gif_dir.join(format!("{}.gif", bpm));

    if !cached_filename.exists() {
        flush_cache(&state.cache_config, &state.cache_gif_dir)
            .await
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

        tracing::debug!("this bpm is not cached yet");
        let speed = bpm / state.source_bpm;
        let output_frames = (state.frame_count as f64 / speed).floor() as usize;
        if output_frames <= 1 {
            return Err(error(
                StatusCode::BAD_REQUEST,
                "Hat Kid got incarcerated for speeding on a highway.",
            ));
        }
        if output_frames > 900 {
            return Err(error(StatusCode::BAD_REQUEST, "yawn…"));
        }

        let mut accumulator: f64 = 0.0;
        let input_filenames: Vec<_> = (0..output_frames)
            .filter_map(|_| {
                let input_frame = accumulator.floor() as usize + 1;
                accumulator += speed;
                let path = Path::new(FRAMES_PATH).join(format!("{input_frame}.png"));
                path.to_str().map(|x| x.to_owned())
            })
            .collect();

        let output_filename = format!("{}.gif", generate_unique_filename(32));
        let output_filename = state.cache_work_dir.join(&output_filename);
        Command::new(&state.gifski)
            .arg("--fps")
            .arg("50")
            .arg("--height")
            .arg("360")
            .arg("--output")
            .arg(&output_filename)
            .arg("--no-sort")
            .args(&input_filenames)
            .spawn()
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?
            .wait()
            .await
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

        tokio::fs::rename(&output_filename, &cached_filename)
            .await
            .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        tracing::debug!("render of {bpm} bpm complete");
    }

    let file = tokio::fs::read(&cached_filename)
        .await
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut response = file.into_response();
    response
        .headers_mut()
        .insert("Content-Type", "image/gif".try_into().unwrap());
    Ok(response)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    tracing::debug!("loading config from {CONFIG_PATH}");
    let config = std::fs::read_to_string(CONFIG_PATH).expect("failed to load config file");
    let config: Config = toml::from_str(&config).expect("config TOML deserialization error");

    let cache_work_dir = config.cache.dir.join("work");
    let cache_gif_dir = config.cache.dir.join("gif");
    std::fs::create_dir_all(&cache_work_dir).expect("cannot create cache/work directory");
    std::fs::create_dir_all(&cache_gif_dir).expect("cannot create cache/gif directory");

    let frame_count = count_frames();
    let minimum_bpm = get_minimum_bpm(frame_count as f64);
    tracing::debug!("found {frame_count} animation frames");
    tracing::debug!("given {WAVE_COUNT} waves at {ANIMATION_FPS} fps, the minimum bpm for playback at full framerate is {minimum_bpm}");

    let state = Arc::new(State {
        index: render_index(IndexData {
            root: config.root,
            minimum_bpm,
        }),
        frame_count,
        source_bpm: minimum_bpm,
        gifski: config.gifski,
        cache_work_dir,
        cache_gif_dir,
        cache_config: config.cache,
    });

    let app = Router::new()
        .route(
            "/",
            get({
                let state = Arc::clone(&state);
                move || index(state)
            }),
        )
        .route(
            "/:bpm",
            get({
                let state = Arc::clone(&state);
                move |bpm| render_animation(state, bpm)
            }),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("failed to start server");
}
