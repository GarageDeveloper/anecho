//! Device discovery: where analyzers come from and how they are described.
//!
//! - [`DeviceSource`]: one place devices come from (the USB bus, or with the
//!   `sim` feature the embedded simulator). `enumerate()` is async, fallible,
//!   side-effect free and returns N descriptors per source; `open()` opens one
//!   unit *by id* onto a shared [`DeviceHandle`] — never "first device on the
//!   bus".
//! - [`Analyzer`]: the narrow control surface (identity, capabilities, ranges,
//!   sample rate, telemetry). It deliberately excludes the acquisition surface,
//!   which stays on the concrete [`crate::QA40xDevice`].
//! - [`DeviceCapabilities`]: the explicit capability record (channels, rates
//!   including the QA403-only 384 kHz, range tables, calibration source).
//!
//! The capability registers 0x1B/0x1C are deliberately not consulted: a real
//! QA403's 0x1B word is unverified, and gating 384 kHz on a register read would
//! be a measurement-semantics change.

pub mod analyzer;
pub mod caps;
pub mod error;
pub mod id;
pub mod source;
pub mod usb;
#[cfg(feature = "sim")]
pub mod virt;

pub use analyzer::Analyzer;
pub use caps::{CalibrationSource, DeviceCapabilities};
pub use error::DeviceError;
pub use id::{DeviceDescriptor, DeviceId, DeviceIdentity, SourceId, SourceKind, Transport};
pub use source::{DeviceHandle, DeviceSource};
pub use usb::{UsbDeviceSource, UsbUnit, classify};
#[cfg(feature = "sim")]
pub use virt::{VirtualDeviceSource, VirtualUnit, demo_unit_options};
