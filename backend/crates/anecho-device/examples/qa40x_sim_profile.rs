//! Energy profile of consecutive capture chunks on the simulated QA40x (feature qa40x-sim).
use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig};
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
    let backend = Qa40xBackend::empty().with_simulator(false);
    let d = backend.enumerate().await.remove(0);
    let dev = backend.open(&d.id).await.unwrap();
    dev.configure(DeviceConfig {
        input_range: Some(1),
        output_range: Some(1),
        ..DeviceConfig::with_sample_rate(48_000)
    })
    .await
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(32);
    let h = dev
        .start(
            StreamConfig {
                block_frames: 512,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine(0.0))),
        )
        .await
        .unwrap();
    let mut line = String::new();
    for i in 0..48 {
        let b = rx.recv().await.unwrap();
        let l: Vec<f32> = b.samples.iter().step_by(2).copied().collect();
        let rms = (l.iter().map(|v| v * v).sum::<f32>() / l.len() as f32).sqrt();
        line.push_str(&format!("{:5.1} ", 20.0 * rms.log10()));
        if (i + 1) % 16 == 0 {
            println!("chunk: {line}");
            line.clear();
        }
    }
    dev.stop(h).await.unwrap();
}
