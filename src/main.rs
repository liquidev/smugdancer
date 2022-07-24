mod common;
mod gifservice;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::Path as UrlPath,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use common::ErrorResponse;
use gifservice::{GifServiceConfig, GifServiceHandle};
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};

use crate::gifservice::GifService;

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

    gif_service: GifServiceConfig,
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

struct State {
    /// The index containing documentation.
    index: String,

    /// The BPM of the source animation.
    source_bpm: f64,

    /// The GIF service.
    gif_service: GifServiceHandle,
}

async fn index(state: Arc<State>) -> Html<String> {
    Html(state.index.clone())
}

async fn render_animation(
    state: Arc<State>,
    UrlPath(unquantized_bpm): UrlPath<f64>,
) -> Result<Response, ErrorResponse> {
    let bpm = quantize_bpm_to_nearest_supported(unquantized_bpm);
    tracing::debug!("serving {bpm} bpm (quantized from {unquantized_bpm} bpm)");

    let speed = bpm / state.source_bpm;
    let file = state
        .gif_service
        .request_speed(speed)
        .await
        .map_err(|e| e.to_response())?;

    let mut response = file.into_response();
    response
        .headers_mut()
        .insert("Content-Type", "image/gif".try_into().unwrap());
    Ok(response)

    // let cached_filename = state.cache_gif_dir.join(format!("{}.gif", bpm));

    // if !cached_filename.exists() {
    //     flush_cache(&state.cache_config, &state.cache_gif_dir)
    //         .await
    //         .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    //     tracing::debug!("this bpm is not cached yet");
    //     let output_frames = (state.frame_count as f64 / speed).floor() as usize;
    //     if output_frames <= 1 {
    //         tracing::debug!("requested bpm is too fast");
    //         return Err(error(
    //             StatusCode::BAD_REQUEST,
    //             "Hat Kid got incarcerated for speeding on a highway.",
    //         ));
    //     }
    //     if output_frames > 900 {
    //         tracing::debug!("requested bpm is too slow");
    //         return Err(error(StatusCode::BAD_REQUEST, "yawn…"));
    //     }

    //     let mut accumulator: f64 = 0.0;
    //     let input_filenames: Vec<_> = (0..output_frames)
    //         .filter_map(|_| {
    //             let input_frame = accumulator.floor() as usize + 1;
    //             accumulator += speed;
    //             let path = Path::new(FRAMES_PATH).join(format!("{input_frame}.png"));
    //             path.to_str().map(|x| x.to_owned())
    //         })
    //         .collect();

    //     let output_filename = format!("{}.gif", generate_unique_filename(32));
    //     let output_filename = state.cache_work_dir.join(&output_filename);
    //     Command::new(&state.gifski)
    //         .arg("--fps")
    //         .arg("50")
    //         .arg("--height")
    //         .arg("360")
    //         .arg("--output")
    //         .arg(&output_filename)
    //         .arg("--no-sort")
    //         .args(&input_filenames)
    //         .spawn()
    //         .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    //         .wait()
    //         .await
    //         .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    //     tokio::fs::rename(&output_filename, &cached_filename)
    //         .await
    //         .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    //     tracing::debug!("render of {bpm} bpm complete");
    // }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    tracing::debug!("loading config from {CONFIG_PATH}");
    let config = std::fs::read_to_string(CONFIG_PATH).expect("failed to load config file");
    let config: Config = toml::from_str(&config).expect("config TOML deserialization error");

    config
        .gif_service
        .create_cache_dirs()
        .expect("cannot create cache directories for GIF service");

    let frame_count = count_frames();
    let minimum_bpm = get_minimum_bpm(frame_count as f64);
    tracing::debug!("found {frame_count} animation frames");
    tracing::debug!("given {WAVE_COUNT} waves at {ANIMATION_FPS} fps, the minimum bpm for playback at full framerate is {minimum_bpm}");

    let gif_service = GifService::spawn(config.gif_service, frame_count);

    let state = Arc::new(State {
        index: render_index(IndexData {
            root: config.root,
            minimum_bpm,
        }),
        source_bpm: minimum_bpm,
        gif_service,
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
