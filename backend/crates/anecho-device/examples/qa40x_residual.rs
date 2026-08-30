//! Hardware diagnostic: fit a 1 kHz sine to each captured block, then inspect the
//! samples that do not fit (inserted foreign data): positions, period, and content.
use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig};
struct Sine(f64, f32);
impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sr: u32) {
        let step = std::f64::consts::TAU * 1000.0 / sr as f64;
        for fr in buf.chunks_exact_mut(channels as usize) {
            let v = (self.1 as f64 * self.0.sin()) as f32;
            fr.iter_mut().for_each(|s| *s = v);
            self.0 = (self.0 + step) % std::f64::consts::TAU;
        }
    }
}
#[tokio::main]
async fn main() {
    let block: usize = 32768;
    let backend = Qa40xBackend::new();
    let d = backend
        .enumerate()
        .await
        .into_iter()
        .find(|d| d.id.as_str().starts_with("qa40x/usb/"))
        .expect("QA40x");
    let dev = backend.open(&d.id).await.unwrap();
    let caps = dev.capabilities();
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
    dev.configure(DeviceConfig {
        input_range: Some(in_idx),
        output_range: Some(out_idx),
        ..DeviceConfig::with_sample_rate(48_000)
    })
    .await
    .unwrap();
    let amp = 0.3873f32;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let h = dev
        .start(
            StreamConfig {
                block_frames: block as u32,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine(0.0, amp))),
        )
        .await
        .unwrap();
    for b in 0..6 {
        let blk = rx.recv().await.unwrap();
        let l: Vec<f64> = blk.samples.iter().step_by(2).map(|v| *v as f64).collect();
        // least squares fit of A sin + B cos at 1 kHz
        let w = std::f64::consts::TAU * 1000.0 / 48000.0;
        let (mut ss, mut sc, mut cc, mut sy, mut cy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (i, &y) in l.iter().enumerate() {
            let (s, c) = ((i as f64 * w).sin(), (i as f64 * w).cos());
            ss += s * s;
            sc += s * c;
            cc += c * c;
            sy += s * y;
            cy += c * y;
        }
        let det = ss * cc - sc * sc;
        let a = (sy * cc - cy * sc) / det;
        let bb = (cy * ss - sy * sc) / det;
        let resid: Vec<f64> = l
            .iter()
            .enumerate()
            .map(|(i, &y)| y - (a * (i as f64 * w).sin() + bb * (i as f64 * w).cos()))
            .collect();
        let sigma = (resid.iter().map(|r| r * r).sum::<f64>() / resid.len() as f64).sqrt();
        let fitted_amp = (a * a + bb * bb).sqrt();
        let bad: Vec<usize> = resid
            .iter()
            .enumerate()
            .filter(|(_, r)| r.abs() > 8.0 * sigma.max(1e-6))
            .map(|(i, _)| i)
            .collect();
        let mut runs: Vec<(usize, usize)> = vec![];
        for &i in &bad {
            if let Some(last) = runs.last_mut()
                && i <= last.1 + 8
            {
                last.1 = i;
            } else {
                runs.push((i, i));
            }
        }
        let rms_db = 20.0
            * (l.iter().map(|v| v * v).sum::<f64>() / l.len() as f64)
                .sqrt()
                .log10()
            + 9.7476;
        println!(
            "block {b}: rms {rms_db:.2} dBV, fitted amp {:.5}, sigma {:.2e}, {} runs: {}",
            fitted_amp,
            sigma,
            runs.len(),
            runs.iter()
                .take(6)
                .map(|(a, b)| format!("{a}..{b}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(&(s0, e0)) = runs.first() {
            let samples: Vec<String> = (s0..(s0 + 6).min(e0 + 1))
                .map(|i| format!("{:+.6}/fit {:+.6}", l[i], l[i] - resid[i]))
                .collect();
            let hex: Vec<String> = (s0..(s0 + 4).min(e0 + 1))
                .map(|i| format!("{:08X}", (l[i] * 2147483648.0) as i32))
                .collect();
            println!(
                "   first run content: {:?} hex {:?}; run length {}",
                samples,
                hex,
                e0 - s0 + 1
            );
            // ratio to stimulus amplitude: peak of the run vs 0.3873
            let peak = (s0..=e0).map(|i| l[i].abs()).fold(0.0, f64::max);
            println!(
                "   run peak {:.5} = {:+.2} dB re stimulus peak",
                peak,
                20.0 * (peak / amp as f64).log10()
            );
        }
    }
    dev.stop(h).await.unwrap();
}
