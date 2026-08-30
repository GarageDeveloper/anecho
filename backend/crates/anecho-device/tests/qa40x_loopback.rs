//! QA40x loopback: what we generate in dBV must come back in dBV through the factory
//! calibration. Runs hardware-free on the embedded simulator (feature `qa40x-sim`) and,
//! ignored by default, on a real unit with outputs wired to inputs.

#![cfg(feature = "qa40x")]

use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{
    DeviceBackend, DeviceConfig, DeviceDescriptor, Direction, OutputSource, Scale, StreamConfig,
};

struct Sine {
    amp: f32,
    phase: f64,
}

impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sample_rate: u32) {
        let step = std::f64::consts::TAU * 1000.0 / sample_rate as f64;
        for frame in buf.chunks_exact_mut(channels as usize) {
            let v = self.amp * self.phase.sin() as f32;
            frame.iter_mut().for_each(|s| *s = v);
            self.phase = (self.phase + step) % std::f64::consts::TAU;
        }
    }
}

/// Returns the (generated dBV, measured dBV) pair for the left channel.
async fn loopback_dbv(backend: &Qa40xBackend, d: &DeviceDescriptor) -> (f32, f32) {
    let dev = backend.open(&d.id).await.unwrap();
    let caps = dev.capabilities();
    assert!(caps.synchronous_io);
    assert!(!caps.input_ranges.is_empty() && !caps.output_ranges.is_empty());

    // Input 6 dBV range, output -2 dBV range: a -20 dBFS sine stays far from clipping.
    let in_idx = caps
        .input_ranges
        .iter()
        .position(|r| r.full_scale_dbv == 6.0)
        .unwrap();
    let out_idx = caps
        .output_ranges
        .iter()
        .position(|r| r.full_scale_dbv == -2.0)
        .unwrap();
    let applied = dev
        .configure(DeviceConfig {
            input_range: Some(in_idx),
            output_range: Some(out_idx),
            ..DeviceConfig::with_sample_rate(48_000)
        })
        .await
        .unwrap();
    assert_eq!(applied.input_range, Some(in_idx));

    let Scale::Volts { dbv_offset: in_off } = dev.scale(Direction::Input) else {
        panic!("QA40x must be volt-scaled")
    };
    let Scale::Volts {
        dbv_offset: out_off,
    } = dev.scale(Direction::Output)
    else {
        panic!("QA40x must be volt-scaled")
    };

    let amp = 0.1f32; // -20 dBFS peak
    let generated_dbv = out_off + 20.0 * (amp / 2f32.sqrt()).log10();

    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let h = dev
        .start(
            StreamConfig {
                block_frames: 4096,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine { amp, phase: 0.0 })),
        )
        .await
        .unwrap();
    // Skip the first chunk (relay settle, lead-in), measure the second.
    let mut blocks = Vec::new();
    while blocks.len() < 4 {
        blocks.push(rx.recv().await.unwrap());
    }
    dev.stop(h).await.unwrap();
    let last = &blocks[3];
    let left: Vec<f32> = last.samples.iter().step_by(2).copied().collect();
    let rms = (left.iter().map(|v| v * v).sum::<f32>() / left.len() as f32).sqrt();
    let measured_dbv = in_off + 20.0 * rms.log10();
    (generated_dbv, measured_dbv)
}

#[cfg(feature = "qa40x-sim")]
#[tokio::test]
async fn simulator_loopback_is_dbv_coherent() {
    let backend = Qa40xBackend::empty().with_simulator(false);
    let devices = backend.enumerate().await;
    assert_eq!(devices.len(), 1, "one simulated unit");
    assert!(devices[0].id.as_str().starts_with("qa40x/virtual/"));
    let (generated, measured) = loopback_dbv(&backend, &devices[0]).await;
    println!("simulator: generated {generated:.2} dBV, measured {measured:.2} dBV");
    assert!(
        (generated - measured).abs() < 0.5,
        "generated {generated} vs measured {measured} dBV"
    );
}

#[tokio::test]
#[ignore = "needs a QA402/QA403 with outputs looped back to inputs"]
async fn hardware_loopback_is_dbv_coherent() {
    let backend = Qa40xBackend::new();
    let devices = backend.enumerate().await;
    let d = devices
        .iter()
        .find(|d| d.id.as_str().starts_with("qa40x/usb/"))
        .expect("a QA40x on the USB bus");
    println!("unit: {} ({})", d.display_name, d.transport);
    let (generated, measured) = loopback_dbv(&backend, d).await;
    println!("hardware: generated {generated:.2} dBV, measured {measured:.2} dBV");
    assert!(
        (generated - measured).abs() < 0.5,
        "generated {generated} vs measured {measured} dBV"
    );
}
