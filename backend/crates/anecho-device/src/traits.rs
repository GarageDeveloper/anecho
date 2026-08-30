use crate::Result;
use crate::types::*;
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Supplies generated audio to a running stream, in the device's own timing.
///
/// Called from the backend's audio thread: implementations must not block, allocate or
/// lock anything held by non-real-time code.
pub trait OutputSource: Send {
    /// Fill `buf` (interleaved, `channels` wide, full scale ±1.0). The buffer arrives
    /// zeroed, so writing nothing yields silence.
    fn fill(&mut self, buf: &mut [f32], channels: u16, sample_rate: u32);
}

/// Silence.
#[derive(Debug, Default)]
pub struct Silence;

impl OutputSource for Silence {
    fn fill(&mut self, _buf: &mut [f32], _channels: u16, _sample_rate: u32) {}
}

/// A device that can be configured, then streamed.
///
/// Object-safe: the engine keeps `Box<dyn MeasurementDevice>`. One stream at a time per
/// device (the QA40x is a single half-duplex pipe; sound cards could do more, but nothing
/// in Anecho needs it yet).
#[async_trait]
pub trait MeasurementDevice: Send + Sync {
    fn descriptor(&self) -> &DeviceDescriptor;

    fn capabilities(&self) -> &Capabilities {
        &self.descriptor().capabilities
    }

    /// Apply sample rate, ranges and channel selection. Fails while a stream is running.
    async fn configure(&self, cfg: DeviceConfig) -> Result<AppliedConfig>;

    /// Currently applied configuration, if any.
    async fn applied_config(&self) -> Option<AppliedConfig>;

    /// Start streaming. Captured blocks go to `input`; if `cfg.generate`, `output` is polled
    /// for audio (silence when `None`). Returns once the backend is running.
    async fn start(
        &self,
        cfg: StreamConfig,
        input: mpsc::Sender<InputBlock>,
        output: Option<Box<dyn OutputSource>>,
    ) -> Result<StreamHandle>;

    /// Stop and release the stream. Idempotent for an already-stopped handle.
    async fn stop(&self, handle: StreamHandle) -> Result<()>;

    /// How to convert samples of a given direction to absolute units, for the applied config.
    fn scale(&self, direction: Direction) -> Scale;

    fn latency(&self) -> LatencyInfo;
}

/// A source of devices: enumerates and opens them.
#[async_trait]
pub trait DeviceBackend: Send + Sync {
    fn kind(&self) -> BackendKind;

    async fn enumerate(&self) -> Vec<DeviceDescriptor>;

    async fn open(&self, id: &DeviceId) -> Result<Box<dyn MeasurementDevice>>;
}
