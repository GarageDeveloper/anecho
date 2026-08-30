//! Print every device seen by the registered backends.

use anecho_device::DeviceRegistry;
use anecho_device::backends::virtual_loopback::VirtualLoopbackBackend;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut registry =
        DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::default()));
    #[cfg(feature = "cpal")]
    {
        registry =
            registry.with_backend(Arc::new(anecho_device::backends::cpal::CpalBackend::new()));
    }
    for d in registry.enumerate().await {
        let c = &d.capabilities;
        println!(
            "{:<50} {:<28} in={} out={} rates={:?} cal={:?}",
            d.id,
            d.display_name,
            c.input_channels,
            c.output_channels,
            c.sample_rates,
            c.calibration
        );
    }
}
