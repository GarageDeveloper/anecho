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
    /// Expose QuantAsylum QA40x units on the USB bus.
    pub qa40x: bool,
    /// Expose the embedded QA40x simulator (needs feature `qa40x-sim`).
    pub qa40x_sim: bool,
}

pub fn build_engine(opts: &BackendOptions) -> Arc<Engine> {
    let mut registry = DeviceRegistry::new();
    if opts.virtual_loopback {
        registry = registry.with_backend(Arc::new(VirtualLoopbackBackend::new(LoopbackOptions {
            realtime: true,
            ..Default::default()
        })));
    }
    #[cfg(feature = "qa40x")]
    if opts.qa40x || opts.qa40x_sim {
        use anecho_device::backends::qa40x::Qa40xBackend;
        #[allow(unused_mut)]
        let mut backend = if opts.qa40x {
            Qa40xBackend::new()
        } else {
            Qa40xBackend::empty()
        };
        if opts.qa40x_sim {
            #[cfg(feature = "qa40x-sim")]
            {
                backend = backend.with_simulator(true);
            }
            #[cfg(not(feature = "qa40x-sim"))]
            log::warn!("QA40x simulator requested but this build lacks feature qa40x-sim");
        }
        registry = registry.with_backend(Arc::new(backend));
    }
    #[cfg(feature = "cpal")]
    if opts.cpal {
        registry =
            registry.with_backend(Arc::new(anecho_device::backends::cpal::CpalBackend::new()));
    }
    Engine::new(registry)
}
