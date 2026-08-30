//! Hardware diagnostic: per-block L/R levels through a QA40x loopback for a few range
//! combinations, plus calibration status. Set `PROBE_DELAY_MS` to pause between configs.

use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, Direction, OutputSource, StreamConfig};

struct Sine(f64);
impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sample_rate: u32) {
        let step = std::f64::consts::TAU * 1000.0 / sample_rate as f64;
        for frame in buf.chunks_exact_mut(channels as usize) {
            let v = 0.1 * self.0.sin() as f32;
            frame.iter_mut().for_each(|s| *s = v);
            self.0 = (self.0 + step) % std::f64::consts::TAU;
        }
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let backend = Qa40xBackend::new();
    let d = backend
        .enumerate()
        .await
        .into_iter()
        .find(|d| d.id.as_str().starts_with("qa40x/usb/"))
        .expect("QA40x");
    println!(
        "{} cal={:?} in_ranges={:?} out_ranges={:?}",
        d.display_name,
        d.capabilities.calibration,
        d.capabilities
            .input_ranges
            .iter()
            .map(|r| r.full_scale_dbv)
            .collect::<Vec<_>>(),
        d.capabilities
            .output_ranges
            .iter()
            .map(|r| r.full_scale_dbv)
            .collect::<Vec<_>>()
    );
    let dev = backend.open(&d.id).await.unwrap();
    println!("after open: cal={:?}", dev.capabilities().calibration);
    let delay_ms: u64 = std::env::var("PROBE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    for (in_dbv, out_dbv) in [
        (6.0, -2.0),
        (42.0, -2.0),
        (6.0, -2.0),
        (0.0, -2.0),
        (18.0, 8.0),
    ] {
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        let caps = dev.capabilities();
        let in_idx = caps
            .input_ranges
            .iter()
            .position(|r| r.full_scale_dbv == in_dbv)
            .unwrap();
        let out_idx = caps
            .output_ranges
            .iter()
            .position(|r| r.full_scale_dbv == out_dbv)
            .unwrap();
        dev.configure(DeviceConfig {
            input_range: Some(in_idx),
            output_range: Some(out_idx),
            ..DeviceConfig::with_sample_rate(48_000)
        })
        .await
        .unwrap();
        println!(
            "in {in_dbv:+} dBV out {out_dbv:+} dBV  scale in={:?} out={:?}",
            dev.scale(Direction::Input),
            dev.scale(Direction::Output)
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let h = dev
            .start(
                StreamConfig {
                    block_frames: 4096,
                    capture: true,
                    generate: true,
                },
                tx,
                Some(Box::new(Sine(0.0))),
            )
            .await
            .unwrap();
        for _ in 0..2 {
            let b = rx.recv().await.unwrap();
            let rms = |ch: usize| {
                let v: Vec<f32> = b.samples.iter().skip(ch).step_by(2).copied().collect();
                20.0 * (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32)
                    .sqrt()
                    .log10()
            };
            let peak = |ch: usize| {
                b.samples
                    .iter()
                    .skip(ch)
                    .step_by(2)
                    .fold(0f32, |m, x| m.max(x.abs()))
            };
            println!(
                "  block {} frame {:>6}: L rms {:7.2} peak {:.4} | R rms {:7.2} peak {:.4} dBFS",
                b.seq,
                b.first_frame,
                rms(0),
                peak(0),
                rms(1),
                peak(1)
            );
        }
        dev.stop(h).await.unwrap();
    }
}
