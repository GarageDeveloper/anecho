//! Unofficial Rust driver for the QuantAsylum QA402 / QA403 USB audio analyzers.
//!
//! No UI, no DSP: this crate speaks the device's USB protocol and exposes the
//! analyzer as a calibrated, range-switched, synchronous generate-and-capture
//! device.
//!
//! - [`QA40xDevice`] — one analyzer: connect, registers, telemetry, input /
//!   output ranges, sample rate, factory calibration page, streaming
//!   (`acquire_data`, `generate_and_capture`), dBFS ↔ dBV conversion.
//! - [`discovery`] — enumeration of units on the USB bus (and, with the `sim`
//!   feature, of the embedded virtual QA40x), capability records, the narrow
//!   [`Analyzer`] control trait.
//! - [`register`], [`transport`], [`i2s`], [`settle`] — the protocol layers.
//!
//! # Features
//! - `serde` (default): serde derives on the public value types.
//! - `ts`: ts-rs derives (TypeScript bindings).
//! - `sim`: the in-process `vqa40x-core` simulator behind the same endpoint
//!   queues as the hardware — hardware-free tests and demos.

pub mod device;
pub mod discovery;
pub mod error;
pub mod i2s;
pub mod register;
pub mod settle;
pub mod transport;
pub mod types;

pub use device::{DeviceMeta, QA40xDevice, Telemetry};
#[cfg(feature = "sim")]
pub use discovery::VirtualDeviceSource;
pub use discovery::{
    Analyzer, CalibrationSource, DeviceCapabilities, DeviceDescriptor, DeviceError, DeviceHandle,
    DeviceId, DeviceIdentity, DeviceSource, SourceId, SourceKind, Transport, UsbDeviceSource,
    classify,
};
pub use error::{QA40xError, Result};
pub use types::*;

pub mod calpage_crc;
pub use calpage_crc::{CALIBRATION_PAGE_LEN, calibration_page_crc_ok, crc16_buypass};
mod debug_impls;
#[cfg(feature = "sim")]
mod debug_impls_sim;
mod nonce;
