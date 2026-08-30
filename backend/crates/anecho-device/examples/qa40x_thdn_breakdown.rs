//! Hardware diagnostic: where does the THD+N residual of a QA40x loopback sit?
//! Captures one bin-centred sine, averages power spectra, and reports the residual power
//! (everything but the fundamental's main lobe) per frequency region, relative to the
//! fundamental.
use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig};
use anecho_dsp::fft::RealSpectrum;
use anecho_dsp::window::Window;

struct Sine {
    hz: f64,
    amp: f64,
    ph: f64,
}
impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sr: u32) {
        let step = std::f64::consts::TAU * self.hz / sr as f64;
        for fr in buf.chunks_exact_mut(channels as usize) {
            let v = (self.amp * self.ph.sin()) as f32;
            fr.iter_mut().for_each(|s| *s = v);
            self.ph = (self.ph + step) % std::f64::consts::TAU;
        }
    }
}

#[tokio::main]
async fn main() {
    let n: usize = std::env::var("N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(65536);
    let window = Window::BlackmanHarris7;
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
    let bin = 48000.0 / n as f64;
    let hz = (1000.0 / bin).round() * bin;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let h = dev
        .start(
            StreamConfig {
                block_frames: n as u32,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Sine {
                hz,
                amp: 0.3,
                ph: 0.0,
            })),
        )
        .await
        .unwrap();
    let mut fft = RealSpectrum::new(n);
    let mut acc = vec![0f64; n / 2 + 1];
    let _ = rx.recv().await.unwrap(); // discard first block
    let avg = 4;
    for _ in 0..avg {
        let b = rx.recv().await.unwrap();
        let left: Vec<f32> = b.samples.iter().step_by(2).copied().collect();
        let p = fft.power(&left, window);
        for (a, v) in acc.iter_mut().zip(p) {
            *a += v / avg as f64;
        }
    }
    dev.stop(h).await.unwrap();
    let k0 = (hz / bin).round() as usize;
    let m = window.main_lobe_bins();
    let p1: f64 = acc[k0 - m..=k0 + m].iter().sum();
    let db = |p: f64| 10.0 * (p / p1).log10();
    let region = |lo: f64, hi: f64| -> f64 {
        let (a, b) = (
            (lo / bin).ceil() as usize,
            ((hi / bin).floor() as usize).min(acc.len() - 1),
        );
        (a..=b)
            .filter(|k| *k + m < k0 || *k > k0 + m)
            .map(|k| acc[k])
            .sum()
    };
    println!(
        "sine {hz:.3} Hz bin-centred (bin {k0}), N={n}, BH7 main lobe ±{m} bins, p1 = 0 dB reference"
    );
    for (name, lo, hi) in [
        ("20 Hz–500 Hz", 20.0, 500.0),
        ("500–950 Hz", 500.0, 950.0),
        ("950–1050 Hz skirt (excl. lobe)", 950.0, 1050.0),
        ("1050–1950 Hz", 1050.0, 1950.0),
        ("harmonics 2..9 ±lobe", 0.0, 0.0),
        ("1950–20000 Hz rest", 1950.0, 20000.0),
        ("total 20 Hz–20 kHz", 20.0, 20000.0),
    ] {
        let p = if name.starts_with("harmonics") {
            (2..=9)
                .map(|i| {
                    let k = (i as f64 * hz / bin).round() as usize;
                    acc[k - m..=k + m].iter().sum::<f64>()
                })
                .sum()
        } else {
            region(lo, hi)
        };
        println!("  {name:<32} {:7.1} dBc", db(p));
    }
    // Lobe width check: power in bins ±6..±20 around k0
    let skirt: f64 = ((k0 - 20)..(k0 - m))
        .chain((k0 + m + 1)..=(k0 + 20))
        .map(|k| acc[k])
        .sum();
    println!(
        "  bins ±6..±20 around f0 (leakage/phase noise) {:7.1} dBc",
        db(skirt)
    );
    for off in [6usize, 8, 10, 15, 20, 40, 80] {
        println!("    bin k0+{off:<3} {:7.1} dBc", db(acc[k0 + off]));
    }
}
