mod common;
mod gifservice;

use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::Path as UrlPath,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use common::ErrorResponse;
use gifservice::{GifServiceConfig, GifServiceHandle};
use handlebars::Handlebars;
use serde::{Deserialize, Serialize};

use crate::{common::error_response, gifservice::GifService};

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
struct TemplateDataConfig {
    root: String,
    minimum_bpm: f64,
}

#[derive(Serialize)]
struct TemplateData {
    #[serde(flatten)]
    config: TemplateDataConfig,
    css: String,
    js: String,
}

#[derive(Clone)]
struct Pages {
    index: String,
    man: String,
}

fn render_index(config: TemplateDataConfig) -> Pages {
    const INDEX_HBS: &str = include_str!("frontend/index.hbs");
    const MAN_HBS: &str = include_str!("frontend/man.hbs");
    const CSS: &str = concat!("<style>", include_str!("frontend/style.css"), "</style>");
    const JS: &str = concat!("<script>", include_str!("frontend/index.js"), "</script>");

    let mut hbs = Handlebars::new();
    hbs.register_template_string("index", INDEX_HBS)
        .expect("error in index.hbs template");
    hbs.register_template_string("man", MAN_HBS)
        .expect("error in man.hbs template");
    hbs.register_template_string("js", JS)
        .expect("error in js template");

    let template_data = TemplateData {
        css: CSS.to_string(),
        js: hbs
            .render("js", &config)
            .expect("cannot render js template"),
        config,
    };

    Pages {
        index: hbs
            .render("index", &template_data)
            .expect("cannot render index template"),
        man: hbs
            .render("man", &template_data)
            .expect("cannot render index template"),
    }
}

struct State {
    /// The index containing documentation.
    pages: Pages,

    /// The BPM of the source animation.
    source_bpm: f64,

    /// The GIF service.
    gif_service: GifServiceHandle,
}

async fn index(Extension(state): Extension<Arc<State>>) -> Html<String> {
    Html(state.pages.index.clone())
}

async fn man(Extension(state): Extension<Arc<State>>) -> Html<String> {
    Html(state.pages.man.clone())
}

async fn render_animation(
    Extension(state): Extension<Arc<State>>,
    UrlPath(query): UrlPath<String>,
) -> Result<Response, ErrorResponse> {
    let query = query.strip_suffix(".gif").unwrap_or(&query);
    let unquantized_bpm: f64 = query.parse().map_err(|e| {
        error_response(
            StatusCode::BAD_REQUEST,
            format!("Cannot parse BPM value: {e}"),
        )
    })?;

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

    let gif_service =
        GifService::spawn(config.gif_service, frame_count).expect("cannot spawn GIF service");

    let state = Arc::new(State {
        pages: render_index(TemplateDataConfig {
            root: config.root,
            minimum_bpm,
        }),
        source_bpm: minimum_bpm,
        gif_service,
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/man", get(man))
        .route("/:query", get(render_animation))
        .layer(Extension(state));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .expect("failed to start server");
}
