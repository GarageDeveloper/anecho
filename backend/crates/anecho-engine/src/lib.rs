//! The headless Anecho engine. Owns devices, sessions and streams; produces ready-to-plot
//! data ([`Frame`]s) so that no client ever computes anything.

pub mod generator;
pub mod levels;

use anecho_device::{
    DeviceConfig, DeviceDescriptor, DeviceError, DeviceId, DeviceRegistry, Direction, InputBlock,
    MeasurementDevice, OutputSource, Scale, StreamConfig, StreamHandle,
};
pub use anecho_wire::Frame;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CONTRACT_VERSION: &str = "v0";

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("no such session {0}")]
    NoSuchSession(u64),
    #[error("no such stream {0}")]
    NoSuchStream(u32),
    #[error("session {0} already has a running stream")]
    StreamRunning(u64),
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error(transparent)]
    Device(#[from] DeviceError),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Levels,
    RawInput,
}

/// Parameters of a stream start.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub kind: StreamKind,
    /// 0 = default (4096).
    pub block_frames: u32,
    /// LEVELS only; 0 = default (20 Hz).
    pub levels_rate_hz: f32,
    pub generator: Option<generator::Signal>,
}

/// What a started stream looks like on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub stream_id: u32,
    pub session_id: u64,
    pub kind: StreamKind,
    pub channels: u16,
    pub sample_rate: u32,
    pub scale: Scale,
    pub values_per_channel: u16,
}

/// Server-side events.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StreamOverrun { stream_id: u32, dropped_blocks: u32 },
    StreamEnded { stream_id: u32 },
}

struct Session {
    device: Arc<dyn MeasurementDevice>,
    stream: Option<RunningStream>,
}

struct RunningStream {
    info: StreamInfo,
    handle: StreamHandle,
    task: JoinHandle<()>,
}

pub struct Engine {
    registry: DeviceRegistry,
    sessions: Mutex<HashMap<u64, Session>>,
    next_session: Mutex<u64>,
    next_stream: Mutex<u32>,
    frames: broadcast::Sender<Arc<Frame>>,
    events: broadcast::Sender<Event>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn new(registry: DeviceRegistry) -> Arc<Self> {
        let (frames, _) = broadcast::channel(256);
        let (events, _) = broadcast::channel(64);
        Arc::new(Self {
            registry,
            sessions: Mutex::new(HashMap::new()),
            next_session: Mutex::new(1),
            next_stream: Mutex::new(1),
            frames,
            events,
        })
    }

    /// Subscribe to every frame of every stream. Slow receivers lag and lose frames.
    pub fn frames(&self) -> broadcast::Receiver<Arc<Frame>> {
        self.frames.subscribe()
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub async fn list_devices(&self) -> Vec<DeviceDescriptor> {
        self.registry.enumerate().await
    }

    pub async fn open_session(
        &self,
        device_id: &DeviceId,
        config: DeviceConfig,
    ) -> Result<(u64, anecho_device::AppliedConfig)> {
        let device = self.registry.open(device_id).await?;
        let applied = device.configure(config).await?;
        let id = {
            let mut n = self.next_session.lock().await;
            let id = *n;
            *n += 1;
            id
        };
        self.sessions.lock().await.insert(
            id,
            Session {
                device: Arc::from(device),
                stream: None,
            },
        );
        Ok((id, applied))
    }

    pub async fn close_session(&self, session_id: u64) -> Result<()> {
        let session = self
            .sessions
            .lock()
            .await
            .remove(&session_id)
            .ok_or(EngineError::NoSuchSession(session_id))?;
        if let Some(rs) = session.stream {
            Self::teardown(&session.device, rs, &self.events).await;
        }
        Ok(())
    }

    pub async fn start_stream(&self, session_id: u64, req: StreamRequest) -> Result<StreamInfo> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(EngineError::NoSuchSession(session_id))?;
        if session.stream.is_some() {
            return Err(EngineError::StreamRunning(session_id));
        }
        let applied = session
            .device
            .applied_config()
            .await
            .ok_or(DeviceError::NotConfigured)?;
        let block_frames = if req.block_frames == 0 {
            4096
        } else {
            req.block_frames
        };
        let channels = applied.input_channels.len() as u16;
        if channels == 0 {
            return Err(EngineError::BadRequest(
                "device has no input channel".into(),
            ));
        }
        let stream_id = {
            let mut n = self.next_stream.lock().await;
            let id = *n;
            *n += 1;
            id
        };
        let scale = session.device.scale(Direction::Input);
        let (values_per_channel, processor): (u16, Processor) = match req.kind {
            StreamKind::Levels => {
                let rate = if req.levels_rate_hz <= 0.0 {
                    20.0
                } else {
                    req.levels_rate_hz
                };
                (
                    2,
                    Processor::Levels(levels::LevelMeter::new(channels, applied.sample_rate, rate)),
                )
            }
            StreamKind::RawInput => (
                u16::try_from(block_frames)
                    .map_err(|_| EngineError::BadRequest("block_frames > 65535".into()))?,
                Processor::Raw,
            ),
        };
        let info = StreamInfo {
            stream_id,
            session_id,
            kind: req.kind,
            channels,
            sample_rate: applied.sample_rate,
            scale,
            values_per_channel,
        };

        let output: Option<Box<dyn OutputSource>> = req
            .generator
            .map(|g| Box::new(generator::Generator::new(g)) as Box<dyn OutputSource>);
        let (tx, rx) = mpsc::channel::<InputBlock>(64);
        let handle = session
            .device
            .start(
                StreamConfig {
                    block_frames,
                    capture: true,
                    generate: output.is_some(),
                },
                tx,
                output,
            )
            .await?;
        let task = tokio::spawn(pump(
            rx,
            stream_id,
            processor,
            self.frames.clone(),
            self.events.clone(),
        ));
        session.stream = Some(RunningStream {
            info: info.clone(),
            handle,
            task,
        });
        Ok(info)
    }

    pub async fn stop_stream(&self, stream_id: u32) -> Result<()> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .values_mut()
            .find(|s| {
                s.stream
                    .as_ref()
                    .is_some_and(|r| r.info.stream_id == stream_id)
            })
            .ok_or(EngineError::NoSuchStream(stream_id))?;
        let rs = session.stream.take().expect("checked above");
        let device = session.device.clone();
        drop(sessions);
        Self::teardown(&device, rs, &self.events).await;
        Ok(())
    }

    /// Stop every stream and close every session (server shutdown).
    pub async fn shutdown(&self) {
        let ids: Vec<u64> = self.sessions.lock().await.keys().copied().collect();
        for id in ids {
            let _ = self.close_session(id).await;
        }
    }

    async fn teardown(
        device: &Arc<dyn MeasurementDevice>,
        rs: RunningStream,
        events: &broadcast::Sender<Event>,
    ) {
        if let Err(e) = device.stop(rs.handle).await {
            log::warn!("stop stream {}: {e}", rs.info.stream_id);
        }
        // Dropping the device-side sender ends the pump naturally; wait for it.
        let _ = rs.task.await;
        let _ = events.send(Event::StreamEnded {
            stream_id: rs.info.stream_id,
        });
    }
}

enum Processor {
    Levels(levels::LevelMeter),
    Raw,
}

async fn pump(
    mut rx: mpsc::Receiver<InputBlock>,
    stream_id: u32,
    mut processor: Processor,
    frames: broadcast::Sender<Arc<Frame>>,
    events: broadcast::Sender<Event>,
) {
    let mut seq: u64 = 0;
    while let Some(block) = rx.recv().await {
        if block.dropped_before > 0 {
            let _ = events.send(Event::StreamOverrun {
                stream_id,
                dropped_blocks: block.dropped_before,
            });
        }
        match &mut processor {
            Processor::Raw => {
                let ch = block.channels as usize;
                let n = block.frames as usize;
                let mut values = vec![0f32; ch * n];
                for (i, frame) in block.samples.chunks_exact(ch).enumerate() {
                    for (c, v) in frame.iter().enumerate() {
                        values[c * n + i] = *v;
                    }
                }
                let _ = frames.send(Arc::new(Frame {
                    stream_id,
                    seq,
                    first_frame: block.first_frame,
                    channels: block.channels,
                    values_per_channel: block.frames as u16,
                    values,
                }));
                seq += 1;
            }
            Processor::Levels(meter) => {
                for reading in meter.push(&block) {
                    let _ = frames.send(Arc::new(Frame {
                        stream_id,
                        seq,
                        first_frame: reading.first_frame,
                        channels: block.channels,
                        values_per_channel: 2,
                        values: reading.values,
                    }));
                    seq += 1;
                }
            }
        }
    }
}
