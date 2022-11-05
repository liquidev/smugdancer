use serde::Deserialize;

use crate::{cache_service::CacheServiceConfig, render_service::RenderServiceConfig};

pub const PATH: &str = "smugdancer.toml";

#[derive(Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub animation: AnimationConfig,
    pub render_service: RenderServiceConfig,
    pub cache_service: CacheServiceConfig,
}

#[derive(Deserialize)]
pub struct AnimationConfig {
    /// The framerate at which the resulting GIF should be rendered. This value is substituted for
    /// the argument `{fps}` in the render command.
    ///
    /// NOTE: 50 fps is a GIF limitation. See index.hbs.
    pub fps: f64,
    /// The number of times Hat Kid waves her hands back and forth in the animation.
    pub wave_count: f64,
    /// The way of obtaining the frame count.
    /// For giffel archives, `Command` should be used running `giffel stat <archive> frame-count`.
    pub frame_count: FrameCountSource,
}

/// Source for obtaining the number of frames in an animation.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum FrameCountSource {
    Hardcoded { hardcoded: usize },
    Command { command: String, flags: Vec<String> },
}

#[derive(Deserialize)]
pub struct ServerConfig {
    /// The port under which smugdancer should serve.
    pub port: u16,
    /// The root URL that's shown on the documentation website.
    pub root: String,
    /// Set to `false` to disable rate limiting. This is available for testing in development
    /// environments, where getting multiple IP addresses to circumvent rate limits is not
    /// practical. On production servers this should be **always** enabled.
    #[serde(default = "enabled")]
    pub rate_limiting: bool,
    /// Set to `true` if the server is behind a reverse proxy like nginx.
    /// This makes it use the X-Forwarded-For header for rate limiting instead of the connection's
    /// IP address.
    #[serde(default)]
    pub reverse_proxy: bool,
}

fn enabled() -> bool {
    true
}
