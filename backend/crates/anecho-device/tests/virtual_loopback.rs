//! The virtual loopback must return what was generated, delayed by its latency.

use anecho_device::backends::virtual_loopback::{LoopbackOptions, VirtualLoopbackBackend};
use anecho_device::{DeviceBackend, DeviceConfig, DeviceRegistry, OutputSource, StreamConfig};
use std::sync::Arc;

struct Sine {
    freq: f32,
    amp: f32,
    phase: f32,
}

impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sample_rate: u32) {
        let step = std::f32::consts::TAU * self.freq / sample_rate as f32;
        for frame in buf.chunks_exact_mut(channels as usize) {
            let v = self.amp * self.phase.sin();
            frame.iter_mut().for_each(|s| *s = v);
            self.phase = (self.phase + step) % std::f32::consts::TAU;
        }
    }
}

#[tokio::test]
async fn sine_comes_back_with_expected_level_and_delay() {
    let latency = 1000u32;
    let registry = DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::new(
        LoopbackOptions {
            latency_frames: latency,
            gain_db: -6.0,
            ..Default::default()
        },
    )));
    let devices = registry.enumerate().await;
    assert_eq!(devices.len(), 1);
    let dev = registry.open(&devices[0].id).await.unwrap();
    let applied = dev
        .configure(DeviceConfig::with_sample_rate(48_000))
        .await
        .unwrap();
    assert_eq!(applied.input_channels, vec![0, 1]);

    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let handle = dev
        .start(
            StreamConfig {
                block_frames: 512,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine {
                freq: 1000.0,
                amp: 0.5,
                phase: 0.0,
            })),
        )
        .await
        .unwrap();

    // Collect ~4 blocks (2048 frames), enough to cover the 1000-frame delay.
    let mut samples: Vec<f32> = Vec::new();
    let mut last_seq = None;
    while samples.len() < 2048 * 2 {
        let b = rx.recv().await.unwrap();
        assert_eq!(b.channels, 2);
        assert_eq!(b.frames, 512);
        assert_eq!(b.dropped_before, 0);
        if let Some(s) = last_seq {
            assert_eq!(b.seq, s + 1);
        }
        last_seq = Some(b.seq);
        samples.extend_from_slice(&b.samples);
    }
    dev.stop(handle).await.unwrap();

    let left: Vec<f32> = samples.iter().step_by(2).copied().collect();
    // Silence before the delay elapses.
    assert!(left[..latency as usize].iter().all(|&v| v == 0.0));
    // Then a sine at 0.5 * -6 dB ≈ 0.2506 peak → RMS ≈ 0.1772.
    let tail = &left[latency as usize..];
    let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
    let expected = 0.5 * 10f32.powf(-6.0 / 20.0) / 2f32.sqrt();
    assert!((rms - expected).abs() < 0.01, "rms {rms} vs {expected}");
    // First non-zero sample is exactly at the latency.
    let first_nonzero = left.iter().position(|&v| v != 0.0).unwrap();
    assert!(first_nonzero >= latency as usize && first_nonzero <= latency as usize + 1);

    // Stopping twice is fine.
    dev.stop(handle).await.unwrap();
}

#[tokio::test]
async fn start_before_configure_fails() {
    let backend = VirtualLoopbackBackend::default();
    let d = backend.enumerate().await.remove(0);
    let dev = backend.open(&d.id).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    assert!(dev.start(StreamConfig::default(), tx, None).await.is_err());
}
