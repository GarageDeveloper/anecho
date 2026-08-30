//! The headless Anecho engine. Owns devices, sessions and streams; produces ready-to-plot
//! data ([`Frame`]s) so that no client ever computes anything.

pub mod analyzers;
pub mod generator;
pub mod levels;
pub mod measure;

pub use analyzers::rta::{RtaAxis, RtaConfig};
pub use analyzers::scope::{ScopeConfig, Trigger};
pub use measure::{ChannelDistortion, MeasureKind, MeasureRequest, MeasureResult};

use anecho_device::{
    DeviceConfig, DeviceDescriptor, DeviceError, DeviceId, DeviceRegistry, Direction, InputBlock,
    MeasurementDevice, OutputSource, Scale, StreamConfig, StreamHandle, StreamUpdate,
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
    Rta,
    Scope,
}

/// Parameters of a stream start.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub kind: StreamKind,
    /// 0 = default (4096).
    pub block_frames: u32,
    /// LEVELS only; 0 = default (20 Hz).
    pub levels_rate_hz: f32,
    pub generator: Option<generator::GeneratorSpec>,
    /// RTA only; `None` = defaults.
    pub rta: Option<RtaConfig>,
    /// SCOPE only; `None` = defaults.
    pub scope: Option<ScopeConfig>,
}

impl StreamRequest {
    pub fn new(kind: StreamKind) -> Self {
        Self {
            kind,
            block_frames: 0,
            levels_rate_hz: 0.0,
            generator: None,
            rta: None,
            scope: None,
        }
    }
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
    /// RTA: frequency of each point.
    pub axis_hz: Vec<f32>,
    /// SCOPE: time of each point from the window start.
    pub axis_seconds: Vec<f32>,
}

/// Server-side events.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StreamOverrun {
        stream_id: u32,
        dropped_blocks: u32,
    },
    StreamEnded {
        stream_id: u32,
    },
    /// The engine changed a range of the session's device (generator level fitting,
    /// input auto-range).
    RangeChanged {
        session_id: u64,
        input_range: Option<usize>,
        output_range: Option<usize>,
    },
}

struct Session {
    device: Arc<dyn MeasurementDevice>,
    stream: Option<RunningStream>,
    /// A one-shot measurement is using the device.
    measuring: bool,
}

struct RunningStream {
    info: StreamInfo,
    handle: StreamHandle,
    task: JoinHandle<()>,
}

pub struct Engine {
    registry: DeviceRegistry,
    sessions: Mutex<HashMap<u64, Session>>,
    /// Devices currently open, shared between sessions: a USB analyzer is claimed
    /// exclusively, so a second session on the same device must reuse the open handle
    /// instead of opening it again. Entries die with their last session.
    open_devices: Mutex<HashMap<DeviceId, std::sync::Weak<dyn MeasurementDevice>>>,
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
            open_devices: Mutex::new(HashMap::new()),
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
    ) -> Result<(u64, anecho_device::AppliedConfig, DeviceDescriptor)> {
        let device: Arc<dyn MeasurementDevice> = {
            let mut open = self.open_devices.lock().await;
            open.retain(|_, w| w.strong_count() > 0);
            match open.get(device_id).and_then(|w| w.upgrade()) {
                Some(shared) => shared,
                None => {
                    let d: Arc<dyn MeasurementDevice> =
                        Arc::from(self.registry.open(device_id).await?);
                    open.insert(device_id.clone(), Arc::downgrade(&d));
                    d
                }
            }
        };
        if config.auto_range_input {
            return Err(EngineError::BadRequest(
                "input auto-range is not available: changing the input range while a QA40x \
                 streams was found to corrupt its captures; set the range explicitly"
                    .into(),
            ));
        }
        let applied = device.configure(config).await?;
        let descriptor = device.descriptor().clone();
        let id = {
            let mut n = self.next_session.lock().await;
            let id = *n;
            *n += 1;
            id
        };
        self.sessions.lock().await.insert(
            id,
            Session {
                device,
                stream: None,
                measuring: false,
            },
        );
        Ok((id, applied, descriptor))
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
        // Last user of this device? Put the hardware back in a safe state (QA40x: safe
        // input range, stream stopped) — every client exit path funnels here.
        let still_used = self
            .sessions
            .lock()
            .await
            .values()
            .any(|s| Arc::ptr_eq(&s.device, &session.device));
        if !still_used {
            session.device.release().await;
        }
        Ok(())
    }

    /// Start a stream on the session. When the session already has one running, the new
    /// stream **reuses the device's persistent capture loop**: the engine updates the block
    /// size / generator / block sink in place ([`MeasurementDevice::update_stream`]) and
    /// the device never stops — rapidly changing parameters therefore never cancels a
    /// fresh capture (the sequence that corrupts a QA402, see the qa40x backend docs).
    /// The previous stream ends (`StreamEnded`), the new one gets a fresh id.
    ///
    /// Exception: a generator whose dBV level needs another output range goes through the
    /// stop → reconfigure → start path — range registers are only written while the
    /// device is idle (measured safe).
    pub async fn start_stream(&self, session_id: u64, req: StreamRequest) -> Result<StreamInfo> {
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or(EngineError::NoSuchSession(session_id))?;
        if session.measuring {
            return Err(EngineError::StreamRunning(session_id));
        }
        let mut applied = session
            .device
            .applied_config()
            .await
            .ok_or(DeviceError::NotConfigured)?;
        let mut block_frames = if req.block_frames == 0 {
            4096
        } else {
            req.block_frames
        };
        // An RTA frame must come from one contiguous capture (the QA40x captures block by
        // block with a gap in between): ask the device for blocks of at least one FFT.
        if req.kind == StreamKind::Rta {
            let fft = req.rta.as_ref().map(|c| c.fft_length as u32).unwrap_or(0);
            block_frames = block_frames.max(fft);
        }
        let channels = applied.input_channels.len() as u16;
        if channels == 0 {
            return Err(EngineError::BadRequest(
                "device has no input channel".into(),
            ));
        }
        // A dBV generator level may need another output range; range registers are only
        // written while the device is idle, so that path stops the running stream first.
        if let Some(spec) = &req.generator
            && let Some(idx) = Self::output_range_for(&session.device, spec)?
            && applied.output_range != Some(idx)
        {
            if let Some(rs) = session.stream.take() {
                Self::teardown(&session.device, rs, &self.events).await;
            }
            session
                .device
                .configure(DeviceConfig {
                    sample_rate: applied.sample_rate,
                    input_range: applied.input_range,
                    output_range: Some(idx),
                    input_channels: applied.input_channels.clone(),
                    output_channels: applied.output_channels.clone(),
                    auto_range_input: false,
                })
                .await?;
            applied = session
                .device
                .applied_config()
                .await
                .ok_or(DeviceError::NotConfigured)?;
            let _ = self.events.send(Event::RangeChanged {
                session_id,
                input_range: None,
                output_range: Some(idx),
            });
        }
        let output: Option<Box<dyn OutputSource>> = match req.generator.clone() {
            Some(spec) => Some(Self::build_generator(&session.device, &applied, spec)?),
            None => None,
        };
        let stream_id = {
            let mut n = self.next_stream.lock().await;
            let id = *n;
            *n += 1;
            id
        };
        let scale = session.device.scale(Direction::Input);
        // Frames are ready to plot: dBFS, or dBV when the device is calibrated.
        let offset_db = match scale {
            Scale::Dbfs => 0.0,
            Scale::Volts { dbv_offset } => dbv_offset,
        };
        let too_many = |n: usize| {
            u16::try_from(n).map_err(|_| EngineError::BadRequest("more than 65535 points".into()))
        };
        let mut axis_hz = Vec::new();
        let mut axis_seconds = Vec::new();
        let (values_per_channel, processor): (u16, Processor) = match req.kind {
            StreamKind::Levels => {
                let rate = if req.levels_rate_hz <= 0.0 {
                    20.0
                } else {
                    req.levels_rate_hz
                };
                (
                    2,
                    Processor::Levels(
                        levels::LevelMeter::new(channels, applied.sample_rate, rate)
                            .with_offset_db(offset_db),
                    ),
                )
            }
            StreamKind::RawInput => (too_many(block_frames as usize)?, Processor::Raw),
            StreamKind::Rta => {
                let cfg = req.rta.unwrap_or_default();
                if !cfg.fft_length.is_power_of_two() || cfg.fft_length < 16 {
                    return Err(EngineError::BadRequest(
                        "fft_length must be a power of two >= 16".into(),
                    ));
                }
                let rta = analyzers::rta::Rta::new(&cfg, channels, applied.sample_rate, offset_db);
                axis_hz = rta.axis_hz().to_vec();
                (too_many(rta.points())?, Processor::Rta(rta))
            }
            StreamKind::Scope => {
                let cfg = req.scope.unwrap_or(ScopeConfig {
                    window_frames: block_frames as usize,
                    points: 0,
                    trigger: None,
                });
                let scope = analyzers::scope::Scope::new(&cfg, channels, applied.sample_rate);
                axis_seconds = scope.axis_seconds();
                (too_many(scope.points())?, Processor::Scope(scope))
            }
        };
        let info = StreamInfo {
            stream_id,
            session_id,
            kind: req.kind,
            channels,
            sample_rate: applied.sample_rate,
            scale,
            values_per_channel,
            axis_hz,
            axis_seconds,
        };

        let (tx, rx) = mpsc::channel::<InputBlock>(64);
        let handle = if let Some(old) = session.stream.take() {
            // Reuse the running device loop: swap block size, generator and sink in place.
            match session
                .device
                .update_stream(
                    old.handle,
                    StreamUpdate {
                        block_frames: Some(block_frames),
                        output: Some(output),
                        input: Some(tx),
                    },
                )
                .await
            {
                Ok(()) => {
                    // The worker drops the old sink at its next iteration; the old pump
                    // then drains and ends.
                    let _ = old.task.await;
                    let _ = self.events.send(Event::StreamEnded {
                        stream_id: old.info.stream_id,
                    });
                    old.handle
                }
                Err(DeviceError::UnsupportedConfig(_)) | Err(DeviceError::NoSuchStream) => {
                    // The backend cannot morph this stream (e.g. a cpal stream started
                    // without an output side): fall back to a drained stop + fresh start.
                    // The generator source was consumed by the failed update; rebuild it.
                    Self::teardown(&session.device, old, &self.events).await;
                    let output: Option<Box<dyn OutputSource>> = match &req.generator {
                        Some(spec) => Some(Self::build_generator(
                            &session.device,
                            &applied,
                            spec.clone(),
                        )?),
                        None => None,
                    };
                    let (tx, rx2) = mpsc::channel::<InputBlock>(64);
                    let handle = session
                        .device
                        .start(
                            StreamConfig {
                                block_frames,
                                capture: true,
                                generate: true,
                            },
                            tx,
                            output,
                        )
                        .await?;
                    let task = tokio::spawn(pump(
                        rx2,
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
                    return Ok(info);
                }
                Err(e) => {
                    Self::teardown(&session.device, old, &self.events).await;
                    return Err(e.into());
                }
            }
        } else {
            session
                .device
                .start(
                    StreamConfig {
                        block_frames,
                        capture: true,
                        generate: true,
                    },
                    tx,
                    output,
                )
                .await?
        };
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

    /// The output range a dBV generator level needs: the lowest range that carries the
    /// signal (crest factor included, 0.5 dB headroom). `None` for dBFS levels or devices
    /// without output ranges. Pure — no device I/O.
    fn output_range_for(
        device: &Arc<dyn MeasurementDevice>,
        spec: &generator::GeneratorSpec,
    ) -> Result<Option<usize>> {
        let generator::GenLevel::DbvRms(dbv) = spec.level else {
            return Ok(None);
        };
        let Scale::Volts { .. } = device.scale(Direction::Output) else {
            return Err(DeviceError::UnsupportedConfig(
                "a dBV level needs a factory-calibrated device; use peak_dbfs".into(),
            )
            .into());
        };
        let ranges = &device.capabilities().output_ranges;
        if ranges.is_empty() {
            return Ok(None);
        }
        let crest_db =
            generator::crest_factor_db(&spec.signal, device.capabilities().sample_rates[0]);
        // A full-scale signal of crest c has RMS = full_scale_dbv + 3.01 − c.
        let needed = dbv + crest_db - 3.0103 + 0.5;
        let mut best: Option<(usize, f32)> = None;
        for (i, r) in ranges.iter().enumerate() {
            if (r.full_scale_dbv as f64) >= needed
                && best.is_none_or(|(_, fs)| r.full_scale_dbv < fs)
            {
                best = Some((i, r.full_scale_dbv));
            }
        }
        match best {
            Some((idx, _)) => Ok(Some(idx)),
            None => Err(DeviceError::UnsupportedConfig(format!(
                "{dbv:.1} dBV RMS with a crest factor of {crest_db:.1} dB exceeds every output range"
            ))
            .into()),
        }
    }

    /// Build the output source for a generator request. The output range must already fit
    /// (see [`Self::output_range_for`]); a dBV level is converted with the current scale.
    fn build_generator(
        device: &Arc<dyn MeasurementDevice>,
        applied: &anecho_device::AppliedConfig,
        spec: generator::GeneratorSpec,
    ) -> Result<Box<dyn OutputSource>> {
        let sample_rate = applied.sample_rate;
        let level = match spec.level {
            generator::GenLevel::PeakDbfs(db) => generator::Level::Dbfs(db),
            generator::GenLevel::DbvRms(dbv) => {
                let Scale::Volts { dbv_offset } = device.scale(Direction::Output) else {
                    return Err(DeviceError::UnsupportedConfig(
                        "a dBV level needs a factory-calibrated device; use peak_dbfs".into(),
                    )
                    .into());
                };
                let crest_db = generator::crest_factor_db(&spec.signal, sample_rate);
                let rms_dbfs = dbv - dbv_offset as f64;
                generator::Level::Dbfs(rms_dbfs + crest_db)
            }
        };
        let mask: Vec<bool> = if spec.output_channels.is_empty() {
            Vec::new()
        } else {
            (0..applied.output_channels.len() as u16)
                .map(|c| spec.output_channels.contains(&c))
                .collect()
        };
        Ok(Box::new(generator::Generator::new(
            spec.signal,
            sample_rate,
            level,
            mask,
        )))
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
    Rta(analyzers::rta::Rta),
    Scope(analyzers::scope::Scope),
}

/// Send one channel-major frame and bump the sequence number.
fn emit(
    frames: &broadcast::Sender<Arc<Frame>>,
    stream_id: u32,
    seq: &mut u64,
    first_frame: u64,
    channels: u16,
    values: Vec<f32>,
) {
    let values_per_channel = (values.len() / channels.max(1) as usize) as u16;
    let _ = frames.send(Arc::new(Frame {
        stream_id,
        seq: *seq,
        first_frame,
        channels,
        values_per_channel,
        values,
    }));
    *seq += 1;
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
        // Every block carries the scale it was captured with (a range may change between
        // two blocks while earlier blocks are still queued): label with the block's own.
        let block_offset = match block.scale {
            Scale::Dbfs => 0.0,
            Scale::Volts { dbv_offset } => dbv_offset,
        };
        match &mut processor {
            Processor::Levels(m) => m.set_offset_db(block_offset),
            Processor::Rta(r) => r.set_offset_db(block_offset),
            Processor::Raw | Processor::Scope(_) => {}
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
                emit(
                    &frames,
                    stream_id,
                    &mut seq,
                    block.first_frame,
                    block.channels,
                    values,
                );
            }
            Processor::Levels(meter) => {
                for r in meter.push(&block) {
                    emit(
                        &frames,
                        stream_id,
                        &mut seq,
                        r.first_frame,
                        block.channels,
                        r.values,
                    );
                }
            }
            Processor::Rta(rta) => {
                for r in rta.push(&block) {
                    emit(
                        &frames,
                        stream_id,
                        &mut seq,
                        r.first_frame,
                        block.channels,
                        r.values,
                    );
                }
            }
            Processor::Scope(scope) => {
                for r in scope.push(&block) {
                    emit(
                        &frames,
                        stream_id,
                        &mut seq,
                        r.first_frame,
                        block.channels,
                        r.values,
                    );
                }
            }
        }
    }
}
