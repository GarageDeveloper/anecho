//! Hardware diagnostic: stream raw blocks from a QA40x loopback and dump anomalies
//! (samples far above the expected sine peak): where they are, how long they last,
//! and their exact 32-bit pattern.
use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig};

/// L = 1 kHz, R = 1.5 kHz at 0.3 peak, so an inserted stimulus is recognisable per channel.
struct Sine(f64);
impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sr: u32) {
        let step = std::f64::consts::TAU * 1000.0 / sr as f64;
        for fr in buf.chunks_exact_mut(channels as usize) {
            fr[0] = (0.3 * self.0.sin()) as f32;
            if fr.len() > 1 {
                fr[1] = (0.3 * (self.0 * 1.5).sin()) as f32;
            }
            self.0 = (self.0 + step) % std::f64::consts::TAU;
        }
    }
}

#[tokio::main]
async fn main() {
    let in_idx: usize = std::env::var("IN")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let blocks: usize = std::env::var("BLOCKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    let backend = Qa40xBackend::new();
    let d = backend
        .enumerate()
        .await
        .into_iter()
        .find(|d| d.id.as_str().starts_with("qa40x/usb/"))
        .expect("QA40x");
    let dev = backend.open(&d.id).await.unwrap();
    let out_idx = dev
        .capabilities()
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
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let h = dev
        .start(
            StreamConfig {
                block_frames: 8192,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine(0.0))),
        )
        .await
        .unwrap();
    // Expected peak in dBFS at this range: 0.3 * 10^((out_off - in_off)/20); anything > 4x is an anomaly.
    let mut bad_blocks = 0;
    for _ in 0..blocks {
        let b = rx.recv().await.unwrap();
        let l: Vec<f32> = b.samples.iter().step_by(2).copied().collect();
        let r: Vec<f32> = b.samples.iter().skip(1).step_by(2).copied().collect();
        let peak = l.iter().fold(0f32, |m, v| m.max(v.abs()));
        let typical = {
            let mut s: Vec<f32> = l.iter().map(|v| v.abs()).collect();
            s.sort_by(|a, b| a.partial_cmp(b).unwrap());
            s[s.len() * 9 / 10]
        };
        if peak > 4.0 * typical.max(1e-6) {
            bad_blocks += 1;
            let idx: Vec<usize> = l
                .iter()
                .enumerate()
                .filter(|(_, v)| v.abs() > 4.0 * typical)
                .map(|(i, _)| i)
                .collect();
            let (first, last) = (idx[0], *idx.last().unwrap());
            let vals: Vec<String> = idx
                .iter()
                .take(6)
                .map(|&i| format!("{}:{:+.5}/{:+.5}", i, l[i], r[i]))
                .collect();
            let hex: Vec<String> = idx
                .iter()
                .take(4)
                .map(|&i| format!("{:08X}", (l[i] as f64 * 2147483648.0) as i32))
                .collect();
            println!(
                "block {} (frame {}): {} anomalous samples, idx {}..{}, typical {:.5}, peak {:.5}; first L/R {:?}; hex {:?}",
                b.seq,
                b.first_frame,
                idx.len(),
                first,
                last,
                typical,
                peak,
                vals,
                hex
            );
        }
    }
    dev.stop(h).await.unwrap();
    println!("{bad_blocks}/{blocks} blocks with anomalies at input range index {in_idx}");
}
