//! Real sound-card loopback through cpal. Ignored by default: needs a device whose outputs
//! feed its inputs (BlackHole on macOS, a physical cable elsewhere). Select it with
//! `ANECHO_LOOPBACK_DEVICE=<substring of the cpal device id>`, default "BlackHole".

#![cfg(feature = "cpal")]

use anecho_device::backends::cpal::CpalBackend;
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig};
use std::time::Duration;

struct Sine(f32);

impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sample_rate: u32) {
        let step = std::f32::consts::TAU * 1000.0 / sample_rate as f32;
        for frame in buf.chunks_exact_mut(channels as usize) {
            let v = 0.25 * self.0.sin();
            frame.iter_mut().for_each(|s| *s = v);
            self.0 = (self.0 + step) % std::f32::consts::TAU;
        }
    }
}

#[tokio::test]
#[ignore = "needs a loopback-capable sound device"]
async fn sine_through_real_loopback() {
    let wanted = std::env::var("ANECHO_LOOPBACK_DEVICE").unwrap_or_else(|_| "BlackHole".into());
    let backend = CpalBackend::new();
    let d = backend
        .enumerate()
        .await
        .into_iter()
        .find(|d| d.id.as_str().contains(&wanted) && d.capabilities.input_channels > 0)
        .unwrap_or_else(|| panic!("no loopback device matching {wanted:?}"));
    let dev = backend.open(&d.id).await.unwrap();
    dev.configure(DeviceConfig::with_sample_rate(48_000))
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let h = dev
        .start(
            StreamConfig {
                block_frames: 1024,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine(0.0))),
        )
        .await
        .unwrap();

    // Skip the first 0.5 s (stream start-up), then average RMS over 1 s.
    let sr = 48_000usize;
    let mut skipped = 0usize;
    let mut sum = 0f64;
    let mut n = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while n < sr {
        let b = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timeout")
            .unwrap();
        if skipped < sr / 2 {
            skipped += b.frames as usize;
            continue;
        }
        for v in b.samples.iter().step_by(b.channels as usize) {
            sum += (*v as f64) * (*v as f64);
            n += 1;
        }
    }
    dev.stop(h).await.unwrap();
    let rms = (sum / n as f64).sqrt();
    let expected = 0.25 / 2f64.sqrt();
    let db = 20.0 * (rms / expected).log10();
    println!("loopback rms {rms:.4} expected {expected:.4} ({db:+.2} dB)");
    assert!(db.abs() < 0.5, "loopback level off by {db:+.2} dB");
}
