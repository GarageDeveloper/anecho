//! Backend implementations.

pub mod blocker;
#[cfg(feature = "cpal")]
pub mod cpal;
pub mod virtual_loopback;
