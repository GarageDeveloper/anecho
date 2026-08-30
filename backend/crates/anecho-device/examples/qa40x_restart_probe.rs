//! Hardware diagnostic: does rapid stop/start of streams (what the app does on every
//! parameter change) put the device in the state where DAC data shows up in the ADC
//! stream? Runs N start→(1 block)→stop cycles through the adapter, then counts
//! stimulus insertions with the raw driver primitive.
//! MODE=cancel (default): stop while the next chunk is in flight. MODE=drain: wait for
//! the chunk to complete before stopping (no cancelled USB transfers).
use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig};
use qa40x_driver::{InputGain, OutputGain, QA40xDevice, SampleRate};

struct Sine(f64);
impl OutputSource for Sine {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sr: u32) {
        let step = std::f64::consts::TAU * 1000.0 / sr as f64;
        for fr in buf.chunks_exact_mut(channels as usize) {
            let v = (0.3 * self.0.sin()) as f32;
            fr.iter_mut().for_each(|s| *s = v);
            self.0 = (self.0 + step) % std::f64::consts::TAU;
        }
    }
}

async fn raw_probe() -> (usize, usize) {
    let dev = QA40xDevice::new();
    dev.connect().await.unwrap();
    if let Some(m) = dev.model().await {
        print!("[{m:?}] ");
    }
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    dev.set_input_gain(InputGain::Gain42dBV).await.unwrap();
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    let n = 8192;
    let left: Vec<f32> = (0..n)
        .map(|i| 0.3 * (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin())
        .collect();
    let mut bad = 0;
    for _ in 0..20 {
        let a = dev.generate_and_capture(&left, &left).await.unwrap();
        if a.left_channel.iter().any(|v| v.abs() > 0.05) {
            bad += 1;
        }
    }
    dev.disconnect().await.unwrap();
    (bad, 20)
}

#[tokio::main]
async fn main() {
    let cycles: usize = std::env::var("CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let mode = std::env::var("MODE").unwrap_or_else(|_| "cancel".into());
    let (b, t) = raw_probe().await;
    println!("baseline: {b}/{t} raw calls with insertions");
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
    for c in 0..cycles {
        let fft = [8192u32, 16384, 32768][c % 3];
        if mode == "inrange" {
            // Input range alternating 42 / 6 dBV between streams (attenuator crossing).
            let i = caps
                .input_ranges
                .iter()
                .position(|r| r.full_scale_dbv == if c % 2 == 0 { 42.0 } else { 6.0 })
                .unwrap();
            dev.configure(DeviceConfig {
                input_range: Some(i),
                output_range: Some(out_idx),
                ..DeviceConfig::with_sample_rate(48_000)
            })
            .await
            .unwrap();
        }
        if mode == "stop-write-start" {
            // E18: stop the stream (cancels the in-flight capture), write the range while
            // idle, restart immediately (no latency probe when SKIP is set).
            let seq: Vec<f32> = std::env::var("SEQ")
                .ok()
                .map(|v| v.split(',').map(|x| x.parse().unwrap()).collect())
                .unwrap_or_else(|| vec![42.0, 24.0, 6.0, 0.0, 18.0, 36.0]);
            for k in 0..6usize {
                let (tx, mut rx) = tokio::sync::mpsc::channel(4);
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
                let _ = rx.recv().await;
                dev.stop(h).await.unwrap();
                let i = caps
                    .input_ranges
                    .iter()
                    .position(|r| r.full_scale_dbv == seq[k % seq.len()])
                    .unwrap();
                dev.set_input_range(i).await.unwrap();
            }
            continue;
        }
        if mode == "inrange-live" {
            // Auto-range style: change the input range while the stream runs.
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
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
            for k in 0..6usize {
                let _ = rx.recv().await;
                let seq: Vec<f32> = std::env::var("SEQ")
                    .ok()
                    .map(|v| v.split(',').map(|x| x.parse().unwrap()).collect())
                    .unwrap_or_else(|| vec![42.0, 24.0, 6.0, 0.0, 18.0, 36.0]);
                let i = caps
                    .input_ranges
                    .iter()
                    .position(|r| r.full_scale_dbv == seq[k % seq.len()])
                    .unwrap();
                dev.set_input_range(i).await.unwrap();
                if let Some(ms) = std::env::var("PAUSE_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    // E8: hold the worker off for a while after the write (the worker is
                    // blocked on the device mutex only during the write itself, so this
                    // pause is enforced by re-taking the range write path: sleep here
                    // does NOT block the worker — it keeps capturing). Instead, stop the
                    // stream, pause, and restart it so no capture starts within `ms` ms.
                    let _ = ms;
                }
            }
            dev.stop(h).await.unwrap();
            continue;
        }
        if mode == "outrange" {
            // What the dBV generator does at every start: output range -12 then -2 dBV.
            let o = caps
                .output_ranges
                .iter()
                .position(|r| r.full_scale_dbv == if c % 2 == 0 { -12.0 } else { -2.0 })
                .unwrap();
            dev.configure(DeviceConfig {
                input_range: Some(in_idx),
                output_range: Some(o),
                ..DeviceConfig::with_sample_rate(48_000)
            })
            .await
            .unwrap();
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let h = dev
            .start(
                StreamConfig {
                    block_frames: fft,
                    capture: true,
                    generate: true,
                },
                tx,
                Some(Box::new(Sine(0.0))),
            )
            .await
            .unwrap();
        let _ = rx.recv().await;
        if mode == "drain" {
            // let the in-flight chunk finish before stopping
            let _ = rx.recv().await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        dev.stop(h).await.unwrap();
    }
    drop(dev);
    println!("after {cycles} start/stop cycles (mode {mode})");
    let (b, t) = raw_probe().await;
    println!("after: {b}/{t} raw calls with insertions");
}
