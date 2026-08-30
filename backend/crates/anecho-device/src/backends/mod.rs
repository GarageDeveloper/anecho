//! Backend implementations.

pub mod blocker;
#[cfg(feature = "cpal")]
pub mod cpal;
#[cfg(feature = "qa40x")]
pub mod qa40x;
pub mod virtual_loopback;
