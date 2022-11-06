#![allow(clippy::or_fun_call)]

mod animation_info;
mod cache_service;
mod common;
mod config;
mod render_service;

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
use cache_service::CacheServiceHandle;
use common::ErrorResponse;
use config::ServerConfig;
use dashmap::DashSet;
use handlebars::Handlebars;
use render_service::RenderService;
use serde::Serialize;
use tracing::{debug, info};

use crate::{
    animation_info::AnimationInfo, cache_service::GifService, common::error_response,
    config::Config,
};

#[derive(Serialize)]
struct TemplateDataConfig {
    root: String,
    minimum_bpm: f64,
}

#[derive(Serialize)]
struct TemplateData {
    #[serde(flatten)]
    config: TemplateDataConfig,
    include_css: String,
    include_js: String,
}

#[derive(Clone)]
struct Pages {
    index: String,
    man: String,
    css: String,
    js: String,
}

fn render_index(config: TemplateDataConfig) -> Pages {
    const INDEX_HBS: &str = include_str!("frontend/index.hbs");
    const MAN_HBS: &str = include_str!("frontend/man.hbs");
    const CSS: &str = include_str!("frontend/style.css");
    const JS: &str = include_str!("frontend/index.js");

    let mut hbs = Handlebars::new();
    hbs.register_template_string("index", INDEX_HBS)
        .expect("error in index.hbs template");
    hbs.register_template_string("man", MAN_HBS)
        .expect("error in man.hbs template");
    hbs.register_template_string("js", JS)
        .expect("error in js template");

    let rendered_js = hbs
        .render("js", &config)
        .expect("cannot render js template");

    let include_css = if cfg!(debug_assertions) {
        r#" <link rel="stylesheet" href="style.css"></link> "#.to_string()
    } else {
        format!("<style>{CSS}</style>")
    };
    let include_js = if cfg!(debug_assertions) {
        r#" <script src="index.js"></script> "#.to_string()
    } else {
        format!("<script>{rendered_js}</script>")
    };

    let template_data = TemplateData {
        include_css,
        include_js,
        config,
    };

    Pages {
        index: hbs
            .render("index", &template_data)
            .expect("cannot render index template"),
        man: hbs
            .render("man", &template_data)
            .expect("cannot render index template"),
        css: CSS.to_owned(),
        js: rendered_js,
    }
}

struct State {
    /// The config file.
    config: ServerConfig,
    /// The info about the animation.
    animation_info: AnimationInfo,
    /// The index containing documentation.
    pages: Pages,
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

async fn js(Extension(state): Extension<Arc<State>>) -> impl IntoResponse {
    (
        [("content-type", "application/javascript")],
        state.pages.js.clone(),
    )
}

async fn css(Extension(state): Extension<Arc<State>>) -> impl IntoResponse {
    ([("content-type", "text/css")], state.pages.css.clone())
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
        let bpm = state
            .animation_info
            .quantize_bpm_to_nearest_supported(unquantized_bpm);
        debug!(
            "serving {bpm} bpm (quantized from {unquantized_bpm} bpm) to {}",
            ip
        );

        let speed = bpm / state.animation_info.minimum_bpm();
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
        debug!(
            "{} (requesting {unquantized_bpm} bpm) is being rate limited",
            ip
        );
        Err(error_response(StatusCode::TOO_MANY_REQUESTS, "Hey you, behave yourself! We only have one Hat Kid, don't spam requests at her like that. Please wait until your previous GIF arrives."))
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config = std::fs::read_to_string(config::PATH).expect("failed to load config file");
    let config: Config = toml::from_str(&config).expect("config TOML deserialization error");
    debug!(path = config::PATH, "loaded config file");

    let animation_info = AnimationInfo::from_config(&config.animation);
    debug!(?animation_info, "resolved animation info");

    let minimum_bpm = animation_info.minimum_bpm();
    debug!(
        minimum_bpm,
        "calculated minimum tempo (given {} waves at {} fps)",
        animation_info.wave_count,
        animation_info.fps
    );

    let render_service = RenderService::spawn(config.render_service, animation_info.clone());
    let gif_service =
        GifService::spawn(config.cache_service, render_service).expect("cannot spawn GIF service");

    let port = config.server.port;
    let state = Arc::new(State {
        animation_info,
        pages: render_index(TemplateDataConfig {
            root: config.server.root.clone(),
            minimum_bpm,
        }),
        config: config.server,
        gif_service,
        waiting_clients: DashSet::new(),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/man", get(man))
        .route("/:query", get(render_animation));
    #[cfg(debug_assertions)]
    let app = app //
        .route("/index.js", get(js))
        .route("/style.css", get(css));
    let app = app.layer(Extension(state));

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("listening on {addr}");
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .expect("failed to start server");
}
