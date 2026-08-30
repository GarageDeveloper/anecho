use crate::types::*;
use crate::{DeviceError, Result};
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

/// In-place changes to a running stream, applied by the backend **between** capture
/// blocks — the device loop never stops for them. Every field is optional; `None` leaves
/// the current value.
#[derive(Default)]
pub struct StreamUpdate {
    /// New size of the emitted [`InputBlock`]s. The block/frame counters restart at 0.
    pub block_frames: Option<u32>,
    /// Replace the output source: `Some(None)` silences the outputs, `Some(Some(_))`
    /// swaps the generator.
    pub output: Option<Option<Box<dyn OutputSource>>>,
    /// Replace the block sink (a new logical stream over the same device loop). The
    /// previous sender is dropped, ending its receiver's stream of blocks.
    pub input: Option<mpsc::Sender<InputBlock>>,
}

impl std::fmt::Debug for StreamUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamUpdate")
            .field("block_frames", &self.block_frames)
            .field("output", &self.output.as_ref().map(|o| o.is_some()))
            .field("input", &self.input.is_some())
            .finish()
    }
}

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
    ///
    /// Backends **drain**: an in-flight capture completes before the call returns.
    /// Cancelling USB transfers early in a stream cycle was measured to corrupt a QA402
    /// persistently (see the qa40x backend documentation), so no backend interrupts a
    /// capture that has started.
    async fn stop(&self, handle: StreamHandle) -> Result<()>;

    /// Reconfigure a running stream in place (block size, generator, block sink), applied
    /// between capture blocks without stopping the device. Backends that cannot honour a
    /// given update return `UnsupportedConfig`; callers then fall back to stop + start.
    async fn update_stream(&self, _handle: StreamHandle, _update: StreamUpdate) -> Result<()> {
        Err(DeviceError::UnsupportedConfig(
            "this device cannot be reconfigured while streaming".into(),
        ))
    }

    /// How to convert samples of a given direction to absolute units, for the applied config.
    fn scale(&self, direction: Direction) -> Scale;

    fn latency(&self) -> LatencyInfo;

    /// Switch the input range **while streaming** (between capture blocks). Backends with
    /// ranges implement it; `scale(Direction::Input)` reflects the new range afterwards.
    async fn set_input_range(&self, _index: usize) -> Result<()> {
        Err(DeviceError::UnsupportedConfig(
            "this device has no switchable input range".into(),
        ))
    }
}

/// A source of devices: enumerates and opens them.
#[async_trait]
pub trait DeviceBackend: Send + Sync {
    fn kind(&self) -> BackendKind;

    async fn enumerate(&self) -> Vec<DeviceDescriptor>;

    async fn open(&self, id: &DeviceId) -> Result<Box<dyn MeasurementDevice>>;
}
