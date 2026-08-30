//! Assembly of the backend: which device backends are registered.

use anecho_device::DeviceRegistry;
use anecho_device::backends::virtual_loopback::{LoopbackOptions, VirtualLoopbackBackend};
use anecho_engine::Engine;
use std::sync::Arc;

/// Options controlling which backends the engine exposes.
#[derive(Debug, Clone, Default)]
pub struct BackendOptions {
    /// Expose the in-process virtual loopback device (tests, demos).
    pub virtual_loopback: bool,
    /// Expose generic sound cards through cpal.
    pub cpal: bool,
}

pub fn build_engine(opts: &BackendOptions) -> Arc<Engine> {
    let mut registry = DeviceRegistry::new();
    if opts.virtual_loopback {
        registry = registry.with_backend(Arc::new(VirtualLoopbackBackend::new(LoopbackOptions {
            realtime: true,
            ..Default::default()
        })));
    }
    #[cfg(feature = "cpal")]
    if opts.cpal {
        registry =
            registry.with_backend(Arc::new(anecho_device::backends::cpal::CpalBackend::new()));
    }
    Engine::new(registry)
}
