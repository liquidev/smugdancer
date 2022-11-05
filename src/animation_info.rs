use std::process::Command;

use tracing::{debug, info_span};

use crate::config::{AnimationConfig, FrameCountSource};

/// Resolved info about an animation.
#[derive(Debug, Clone)]
pub struct AnimationInfo {
    pub fps: f64,
    pub wave_count: f64,
    pub frame_count: usize,
}

impl AnimationInfo {
    /// Resolves animation info from the given config.
    pub fn from_config(config: &AnimationConfig) -> Self {
        Self {
            fps: config.fps,
            wave_count: config.wave_count,
            frame_count: config.frame_count.resolve(),
        }
    }

    pub fn minimum_bpm(&self) -> f64 {
        self.wave_count * self.fps * 60.0 / self.frame_count as f64
    }

    pub fn quantize_bpm_to_nearest_supported(&self, bpm: f64) -> f64 {
        let unrounded_frame_count = self.wave_count * self.fps * 60.0 / bpm;
        let frame_count = unrounded_frame_count.floor();
        self.wave_count * self.fps * 60.0 / frame_count
    }
}

impl FrameCountSource {
    pub fn resolve(&self) -> usize {
        let _span = info_span!("resolve_frame_count");
        match self {
            FrameCountSource::Hardcoded { hardcoded } => *hardcoded,
            FrameCountSource::Command { command, flags } => {
                debug!(
                    ?command,
                    ?flags,
                    "resolving frame count from external program"
                );
                let output = Command::new(command)
                    .args(flags)
                    .output()
                    .expect("failed to run command for determining the frame count");
                assert!(
                    output.status.success(),
                    "frame count command returned with non-zero exit code"
                );
                String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse()
                    .expect("cannot parse frame count command output as a number")
            }
        }
    }
}
