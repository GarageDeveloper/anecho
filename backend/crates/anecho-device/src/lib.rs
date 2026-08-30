//! Measurement device abstraction for Anecho.
//!
//! A [`MeasurementDevice`] is anything that can capture and/or generate audio blocks with a
//! known sample rate and a known scale: a factory-calibrated analyzer (QuantAsylum QA40x) or
//! a generic sound card (cpal). The engine only ever talks to this trait; backends live in
//! [`backends`].
//!
//! Streaming model: block-based, push. Captured blocks are sent on a Tokio channel as
//! [`InputBlock`]s; generated audio is pulled from an [`OutputSource`] in the device's own
//! timing. Both sides are interleaved `f32` in dBFS (full scale = ±1.0); [`Scale`] tells how
//! to convert to volts when the device is calibrated.

pub mod backends;
pub mod error;
pub mod registry;
pub mod traits;
pub mod types;

pub use error::DeviceError;
pub use registry::DeviceRegistry;
pub use traits::{DeviceBackend, MeasurementDevice, OutputSource};
pub use types::*;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, DeviceError>;
