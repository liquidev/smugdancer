use std::{ffi::OsString, path::PathBuf, process::Stdio, sync::Arc};

use dashmap::DashMap;
use serde::Deserialize;
use tokio::{
    process::Command,
    sync::{mpsc, oneshot, Semaphore},
};
use tracing::{debug, error, info, instrument, trace};

use crate::{animation_info::AnimationInfo, common::Error};

#[derive(Deserialize, Clone)]
pub struct RenderServiceConfig {
    /// The path to the encoder executable.
    pub encoder: PathBuf,
    /// Flags to pass onto the encoder. Among these flags must be one whose contents are
    /// `{input_filenames}`, which is expanded to a list of filenames for the encoder.
    pub encoder_flags: Vec<String>,
    /// The maximum number of encoding jobs that are allowed to run at a time.
    pub max_jobs: usize,
}

pub struct RenderService {
    config: RenderServiceConfig,
    animation_info: AnimationInfo,
    queues: DashMap<u64, Vec<oneshot::Sender<RenderResult>>>,
    render_requests: mpsc::Sender<f64>,
    render_jobs: Semaphore,
}

impl RenderService {
    pub fn spawn(
        config: RenderServiceConfig,
        animation_info: AnimationInfo,
    ) -> RenderServiceHandle {
        let (requests_tx, mut requests_rx) = mpsc::channel(32);
        let (renders_tx, mut renders_rx) = mpsc::channel(32);
        let (completed_renders_tx, mut completed_renders_rx) = mpsc::channel(8);

        let service = Arc::new(RenderService {
            animation_info,
            queues: DashMap::new(),
            render_requests: renders_tx,
            render_jobs: Semaphore::new(config.max_jobs),
            config,
        });
        tokio::spawn({
            let service = Arc::clone(&service);
            async move {
                info!("render management task is ready");
                loop {
                    trace!("waiting for messages from threads");
                    tokio::select! {
                        Some(request) = requests_rx.recv() => service.handle_request(request).await,
                        Some((speed, result)) = completed_renders_rx.recv() => {
                            service.handle_complete_render(speed, result).await
                        },
                    }
                }
            }
        });
        tokio::spawn(async move {
            info!("render task is ready");
            // NOTE: Render requests are not handled in separate threads (yet.)
            while let Some(speed) = renders_rx.recv().await {
                trace!(speed, "got render request");
                let completed_renders_tx = completed_renders_tx.clone();
                let service = Arc::clone(&service);
                tokio::spawn(async move {
                    // Should be fine if we discard the error.
                    let _ = completed_renders_tx
                        .send((speed, service.render_speed(speed).await))
                        .await;
                });
            }
        });

        RenderServiceHandle {
            requests: requests_tx,
        }
    }

    async fn handle_request(&self, request: QueueRequest) {
        let QueueRequest { speed, responder } = request;
        trace!(speed, "got queue request");

        let mut queue = self.queues.entry(speed.to_bits()).or_default();
        let request_render = queue.is_empty();
        queue.push(responder);
        if request_render {
            trace!("queue is empty, sending render request");
            self.render_requests
                .send(speed)
                .await
                .expect("render task ended");
            drop(queue);
        }
    }

    async fn handle_complete_render(&self, speed: f64, result: Result<Vec<u8>, Error>) {
        let result = result.map_err(Arc::new);
        // This should *hopefully* lock the map for the entire duration of the function, as well
        // as holding the same lock while removing the item.
        self.queues.remove_if_mut(&speed.to_bits(), |_, queue| {
            for (i, waiting) in queue.drain(..).enumerate() {
                // Ignore error if waiting channel is closed.
                let _ = waiting.send(result.clone().map(|file| (file, i)));
            }
            true
        });
    }

    #[instrument(level = "debug", name = "render", skip(self))]
    async fn render_speed(&self, speed: f64) -> Result<Vec<u8>, Error> {
        // The permit must be given here because we never close the semaphore, thus it is
        // safe to unwrap.
        let _permit = self.render_jobs.acquire().await.unwrap();

        debug!("starting render");

        let output_frames = (self.animation_info.frame_count as f64 / speed).floor() as usize;
        if output_frames <= 1 {
            debug!("requested speed is too fast");
            return Err(Error::SpeedTooFast);
        }
        if output_frames > self.animation_info.frame_count {
            debug!("requested speed is too slow");
            return Err(Error::SpeedTooSlow);
        }

        let args = {
            let mut args = vec![];
            for flag in &self.config.encoder_flags {
                if flag.contains("{frame_indices}") {
                    let mut accumulator: f64 = 0.0;
                    args.extend((0..output_frames).map(|_| {
                        let input_frame = accumulator.floor() as usize + 1;
                        accumulator += speed;
                        flag.replace("{frame_indices}", &input_frame.to_string())
                            .into()
                    }));
                } else if flag.contains("{fps}") {
                    args.push(OsString::from(self.animation_info.fps.to_string()))
                } else {
                    args.push(OsString::from(flag));
                }
            }
            args
        };
        trace!(
            ?self.config.encoder,
            ?args,
            "starting render job",
        );
        let output = Command::new(&self.config.encoder)
            .stdout(Stdio::piped())
            .args(&args)
            .spawn()
            .map_err(Error::Encoder)?
            .wait_with_output()
            .await
            .map_err(Error::Encoder)?;

        if !output.status.success() {
            error!(exit_code = ?output.status, "encoder finished with a non-zero exit code");
            return Err(Error::EncoderExitCode);
        }

        debug!("render complete");

        Ok(output.stdout)
    }
}

type RenderResult = Result<(Vec<u8>, usize), Arc<Error>>;

struct QueueRequest {
    speed: f64,
    responder: oneshot::Sender<RenderResult>,
}

pub struct RenderServiceHandle {
    requests: mpsc::Sender<QueueRequest>,
}

impl RenderServiceHandle {
    /// On success, returns the encoded GIF file and the requester's position in the queue.
    pub async fn render_speed(&self, speed: f64) -> RenderResult {
        let (tx, rx) = oneshot::channel();
        self.requests
            .send(QueueRequest {
                speed,
                responder: tx,
            })
            .await
            .map_err(|_| Error::EncodingJobExited)
            .expect("render service quit unexpectedly");
        rx.await.map_err(|_| Error::EncodingJobExited)?
    }
}
