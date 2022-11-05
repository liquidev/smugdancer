#![allow(clippy::or_fun_call)]

mod cacheservice;
mod common;
mod config;
mod renderservice;

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
use cacheservice::{CacheServiceConfig, CacheServiceHandle};
use common::ErrorResponse;
use dashmap::DashSet;
use handlebars::Handlebars;
use renderservice::{RenderService, RenderServiceConfig};
use serde::{Deserialize, Serialize};

use crate::{cacheservice::GifService, common::error_response};

const CONFIG_PATH: &str = "smugdancer.toml";

#[derive(Deserialize)]
struct Config {
    server: ServerConfig,
    render_service: RenderServiceConfig,
    cache_service: CacheServiceConfig,
}

#[derive(Deserialize)]
struct AnimationConfig {
    /// The framerate at which the resulting GIF should be rendered. This value is substituted for
    /// the argument `{fps}` in the render command.
    ///
    /// NOTE: 50 fps is a GIF limitation. See index.hbs.
    fps: f64,
    /// The number of times Hat Kid waves her hands back and forth in the animation.
    wave_count: f64,
    /// The way of obtaining the frame count.
    /// For giffel archives, `Command` should be used running `giffel stat <archive> frame-count`.
    frame_count: FrameCount,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FrameCount {
    Hardcoded { hardcoded: usize },
    Directory { directory: String },
    Command { command: String, flags: Vec<String> },
}

#[derive(Deserialize)]
struct ServerConfig {
    /// The port under which smugdancer should serve.
    port: u16,
    /// The root URL that's shown on the documentation website.
    root: String,
    /// Set to `false` to disable rate limiting. This is available for testing in development
    /// environments, where getting multiple IP addresses to circumvent rate limits is not
    /// practical. On production servers this should be **always** enabled.
    #[serde(default = "enabled")]
    rate_limiting: bool,
    /// Set to `true` if the server is behind a reverse proxy like nginx.
    /// This makes it use the X-Forwarded-For header for rate limiting instead of the connection's
    /// IP address.
    #[serde(default)]
    reverse_proxy: bool,
}

fn enabled() -> bool {
    true
}

/// Resolved info about an animation.
struct AnimationInfo {}

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
    config: ServerConfig,
    /// The index containing documentation.
    pages: Pages,
    /// The BPM of the source animation.
    source_bpm: f64,
    /// The GIF service.
    gif_service: CacheServiceHandle,
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
            .get("x-forwarded-for")
            .and_then(|val| {
                IpAddr::from_str(
                    val.to_str()
                        .expect("cannot parse X-Forwarded-For IP address"),
                )
                .ok()
            })
            .unwrap_or(addr.ip())
    } else {
        addr.ip()
    };

    if !state.config.rate_limiting || state.waiting_clients.insert(ip) {
        // WARNING: DO NOT USE THE `?` OPERATOR UNTIL THE CLIENT IS REMOVED FROM THE WAIT LIST!!!
        let bpm = quantize_bpm_to_nearest_supported(unquantized_bpm);
        tracing::debug!(
            "serving {bpm} bpm (quantized from {unquantized_bpm} bpm) to {}",
            ip
        );

        let speed = bpm / state.source_bpm;
        let result = state
            .gif_service
            .request_speed(speed)
            .await
            .map_err(|e| e.to_response());
        state.waiting_clients.remove(&ip);
        // It is safe to use the `?` operator from here onward.
        let file = result?;

        let mut response = file.into_response();
        response
            .headers_mut()
            .insert("Content-Type", "image/gif".try_into().unwrap());
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

    let frame_count = count_frames();
    let minimum_bpm = get_minimum_bpm(frame_count as f64);
    tracing::debug!("found {frame_count} animation frames");
    tracing::debug!("given {WAVE_COUNT} waves at {ANIMATION_FPS} fps, the minimum bpm for playback at full framerate is {minimum_bpm}");

    let render_service = RenderService::spawn(config.render_service, frame_count);
    let gif_service =
        GifService::spawn(config.cache_service, render_service).expect("cannot spawn GIF service");

    let port = config.server.port;
    let state = Arc::new(State {
        pages: render_index(TemplateDataConfig {
            root: config.server.root.clone(),
            minimum_bpm,
        }),
        config: config.server,
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
