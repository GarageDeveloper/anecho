//! Plain data types shared by every backend. Kept free of backend-specific details so that
//! the engine and the API layer can map them to the contract without knowing the backend.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Stable, human-readable device identifier: `<backend>/<unit>`, e.g. `qa40x/QA403-1234`
/// or `cpal/coreaudio/MacBook Pro Microphone`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl DeviceId {
    pub fn new(backend: BackendKind, unit: &str) -> Self {
        Self(format!("{}/{}", backend.prefix(), unit))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn backend(&self) -> Option<BackendKind> {
        BackendKind::from_prefix(self.0.split('/').next()?)
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which backend owns a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BackendKind {
    /// QuantAsylum QA402/QA403 through `qa40x-driver` (native USB, factory calibration).
    Qa40x,
    /// Generic sound card through cpal (WASAPI, ASIO, Core Audio, ALSA...).
    Cpal,
    /// In-process virtual loopback used by tests.
    Virtual,
}

impl BackendKind {
    pub fn prefix(self) -> &'static str {
        match self {
            BackendKind::Qa40x => "qa40x",
            BackendKind::Cpal => "cpal",
            BackendKind::Virtual => "virtual",
        }
    }
    pub fn from_prefix(s: &str) -> Option<Self> {
        match s {
            "qa40x" => Some(BackendKind::Qa40x),
            "cpal" => Some(BackendKind::Cpal),
            "virtual" => Some(BackendKind::Virtual),
            _ => None,
        }
    }
}

/// What an enumeration returns: enough to show a picker and open the device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceDescriptor {
    pub id: DeviceId,
    pub display_name: String,
    pub backend: BackendKind,
    /// Free-form transport detail (USB bus/port, audio host name...).
    pub transport: String,
    pub capabilities: Capabilities,
}

/// Whether absolute units are known for this device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Calibration {
    /// Factory calibration read from the device (QA40x): samples map to volts.
    Factory { source: String },
    /// No calibration: samples are dBFS only. A user calibration may be applied upstream.
    None,
}

/// One selectable input or output range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Range {
    /// Full-scale level of the range, in dBV (RMS of a full-scale sine).
    pub full_scale_dbv: f32,
    pub label: String,
}

/// Static capabilities of a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub sample_rates: Vec<u32>,
    pub input_channels: u16,
    pub output_channels: u16,
    pub calibration: Calibration,
    /// Empty when the device has a single fixed (or unknown) range.
    pub input_ranges: Vec<Range>,
    pub output_ranges: Vec<Range>,
    /// True when generation and acquisition are sample-synchronous by construction (QA40x).
    /// Sound cards usually need a loopback timing reference instead.
    pub synchronous_io: bool,
    /// Round-trip latency reported by the backend, if known, in frames at the current rate.
    pub nominal_latency_frames: Option<u32>,
}

/// Requested device configuration. `None` keeps the backend default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub sample_rate: u32,
    /// Index into `Capabilities::input_ranges`.
    pub input_range: Option<usize>,
    /// Index into `Capabilities::output_ranges`.
    pub output_range: Option<usize>,
    /// Physical input channels to capture, in block order. Empty = all.
    pub input_channels: Vec<u16>,
    /// Physical output channels to drive, in block order. Empty = all.
    pub output_channels: Vec<u16>,
    /// Let the engine pick the input range from the signal (devices with ranges). Backends
    /// ignore it; the engine implements the policy.
    pub auto_range_input: bool,
}

impl DeviceConfig {
    pub fn with_sample_rate(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            input_range: None,
            output_range: None,
            input_channels: Vec::new(),
            output_channels: Vec::new(),
            auto_range_input: false,
        }
    }
}

/// What the backend actually applied (ranges resolved, channel lists expanded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppliedConfig {
    pub sample_rate: u32,
    pub input_range: Option<usize>,
    pub output_range: Option<usize>,
    pub input_channels: Vec<u16>,
    pub output_channels: Vec<u16>,
}

/// Per-stream parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamConfig {
    /// Frames per [`InputBlock`]. Backends regroup their native buffers to honour this.
    pub block_frames: u32,
    pub capture: bool,
    pub generate: bool,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            block_frames: 4096,
            capture: true,
            generate: false,
        }
    }
}

/// A captured block: interleaved `f32`, dBFS (±1.0 = full scale of the selected range).
#[derive(Debug, Clone)]
pub struct InputBlock {
    /// Monotonic block counter for this stream, starting at 0.
    pub seq: u64,
    /// Index of the first frame of this block since the stream started.
    pub first_frame: u64,
    pub channels: u16,
    pub frames: u32,
    pub samples: Arc<[f32]>,
    /// Number of blocks the backend had to drop before this one (0 = none).
    pub dropped_before: u32,
    /// Input scale the block was captured with. Carried per block so that a range change
    /// applied between two blocks never mislabels data already captured.
    pub scale: Scale,
}

/// Opaque handle to a running stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamHandle(pub u64);

/// How to interpret sample values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Scale {
    /// Only dBFS is meaningful.
    Dbfs,
    /// `dBV = dBFS + dbv_offset` for RMS values (factory-calibrated devices).
    Volts { dbv_offset: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Input,
    Output,
}

/// Latency knowledge for a device.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LatencyInfo {
    /// Frames of round-trip latency as reported by the backend / driver.
    pub reported_frames: Option<u32>,
    /// Frames measured by a loopback measurement (phase 2), sub-sample precision.
    pub measured_frames: Option<f64>,
}
