#![allow(clippy::or_fun_call)]

mod common;
mod gifservice;

use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use axum::{
    extract::{ConnectInfo, Path as UrlPath},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use common::ErrorResponse;
use dashmap::DashSet;
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
    /// Set to `true` if the server is behind a reverse proxy like nginx.
    /// This makes it use the X-Forwarded-For header for rate limiting instead of the connection's
    /// IP address.
    #[serde(default)]
    reverse_proxy: bool,

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
    /// The config file.
    config: Config,
    /// The index containing documentation.
    pages: Pages,
    /// The BPM of the source animation.
    source_bpm: f64,
    /// The GIF service.
    gif_service: GifServiceHandle,
    /// A map of IP addresses that are currently waiting in the render queue. These IPs will be
    /// rate limited so as not to kill the server with requests.
    waiting_clients: DashSet<IpAddr>,
}

async fn index(Extension(state): Extension<Arc<State>>) -> Html<String> {
    Html(state.pages.index.clone())
}

async fn man(Extension(state): Extension<Arc<State>>) -> Html<String> {
    Html(state.pages.man.clone())
}

async fn render_animation(
    Extension(state): Extension<Arc<State>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    UrlPath(query): UrlPath<String>,
) -> Result<Response, ErrorResponse> {
    let query = query.strip_suffix(".gif").unwrap_or(&query);
    let unquantized_bpm: f64 = query.parse().map_err(|e| {
        error_response(
            StatusCode::BAD_REQUEST,
            format!("Cannot parse BPM value: {e}"),
        )
    })?;

    let ip = if state.config.reverse_proxy {
        headers
            .get("X-Forwarded-For")
            .and_then(|val| IpAddr::from_str(val.to_str().unwrap()).ok())
            .unwrap_or(addr.ip())
    } else {
        addr.ip()
    };

    if state.waiting_clients.insert(ip) {
        let bpm = quantize_bpm_to_nearest_supported(unquantized_bpm);
        tracing::debug!(
            "serving {bpm} bpm (quantized from {unquantized_bpm} bpm) to {}",
            ip
        );

        tracing::debug!("X-Forwarded-For: {:?}", headers.get("X-Forwarded-For"));

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
        state.waiting_clients.remove(&ip);
        Ok(response)
    } else {
        tracing::debug!(
            "{} (requesting {unquantized_bpm} bpm) is being rate limited",
            ip
        );
        Err(error_response(StatusCode::TOO_MANY_REQUESTS, "Hey you, behave yourself! We only have one Hat Kid, don't spam requests at her like that. Please wait until your previous GIF arrives."))
    }
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

    let gif_service = GifService::spawn(config.gif_service.clone(), frame_count)
        .expect("cannot spawn GIF service");

    let port = config.port;
    let state = Arc::new(State {
        pages: render_index(TemplateDataConfig {
            root: config.root.clone(),
            minimum_bpm,
        }),
        config,
        source_bpm: minimum_bpm,
        gif_service,
        waiting_clients: DashSet::new(),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/man", get(man))
        .route("/:query", get(render_animation))
        .layer(Extension(state));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("failed to start server");
}
