//! Hardware diagnostic: per-block L/R levels through a QA40x loopback for a list of
//! (input range, output range) pairs, plus calibration status.
//! `PROBE_PAIRS="42,-2;6,-12"` selects the pairs; `PROBE_DELAY_MS` pauses between configs.

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

fn pairs() -> Vec<(f32, f32)> {
    match std::env::var("PROBE_PAIRS") {
        Ok(v) => v
            .split(';')
            .map(|p| {
                let (a, b) = p.split_once(',').expect("in,out");
                (a.trim().parse().unwrap(), b.trim().parse().unwrap())
            })
            .collect(),
        Err(_) => vec![(6.0, -2.0), (42.0, -2.0), (0.0, -2.0), (18.0, 8.0)],
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
        .expect("QA40x on USB");
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
    let delay_ms: u64 = std::env::var("PROBE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    for (in_dbv, out_dbv) in pairs() {
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        let caps = dev.capabilities();
        let in_idx = caps
            .input_ranges
            .iter()
            .position(|r| r.full_scale_dbv == in_dbv)
            .expect("input range");
        let out_idx = caps
            .output_ranges
            .iter()
            .position(|r| r.full_scale_dbv == out_dbv)
            .expect("output range");
        dev.configure(DeviceConfig {
            input_range: Some(in_idx),
            output_range: Some(out_idx),
            ..DeviceConfig::with_sample_rate(48_000)
        })
        .await
        .unwrap();
        let (in_off, out_off) = match (dev.scale(Direction::Input), dev.scale(Direction::Output)) {
            (
                anecho_device::Scale::Volts { dbv_offset: i },
                anecho_device::Scale::Volts { dbv_offset: o },
            ) => (i, o),
            _ => unreachable!(),
        };
        let expected_dbv = out_off + 20.0 * (0.1f32 / 2f32.sqrt()).log10();
        println!(
            "in {in_dbv:+} dBV out {out_dbv:+} dBV  in_off {in_off:.2} out_off {out_off:.2}  expected {expected_dbv:.2} dBV  latency {:?}",
            dev.latency().measured_frames
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
        for _ in 0..3 {
            let b = rx.recv().await.unwrap();
            let rms = |ch: usize| {
                let v: Vec<f32> = b.samples.iter().skip(ch).step_by(2).copied().collect();
                20.0 * (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32)
                    .sqrt()
                    .log10()
            };
            println!(
                "  block {} frame {:>6}: L {:7.2} dBFS = {:7.2} dBV | R {:7.2} dBFS = {:7.2} dBV",
                b.seq,
                b.first_frame,
                rms(0),
                rms(0) + in_off,
                rms(1),
                rms(1) + in_off
            );
        }
        dev.stop(h).await.unwrap();
        println!(
            "  latency after stream: {:?}",
            dev.latency().measured_frames
        );
    }
}
