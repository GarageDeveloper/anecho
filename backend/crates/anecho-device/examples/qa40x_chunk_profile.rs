//! Hardware diagnostic: RMS profile inside long captures (is the level flat over a call?).
use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, Direction, OutputSource, Scale, StreamConfig};
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
    let block: usize = std::env::var("BLOCK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32768);
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
    let (in_off, out_off) = match (dev.scale(Direction::Input), dev.scale(Direction::Output)) {
        (Scale::Volts { dbv_offset: i }, Scale::Volts { dbv_offset: o }) => (i, o),
        _ => unreachable!(),
    };
    // -10 dBV RMS sine: peak dBFS = -10 - out_off + 3.01
    let amp = 10f32.powf((-10.0 - out_off + 3.0103) / 20.0);
    println!(
        "block {block}, stimulus peak {:.4} (expect {:.2} dBV RMS on input)",
        amp, -10.0
    );
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
    for b in 0..3 {
        let blk = rx.recv().await.unwrap();
        let l: Vec<f32> = blk.samples.iter().step_by(2).copied().collect();
        let seg = 4096;
        let prof: Vec<String> = l
            .chunks(seg)
            .map(|c| {
                let r = (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
                format!("{:6.2}", 20.0 * r.log10() + in_off)
            })
            .collect();
        println!("block {b}: per-{seg} RMS dBV: {}", prof.join(" "));
    }
    dev.stop(h).await.unwrap();
}
