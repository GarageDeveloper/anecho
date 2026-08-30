//! Hardware diagnostic on the raw driver primitive: does `generate_and_capture` return
//! stretches of the stimulus in place of ADC data, and does it depend on call length?
//! Input range 42 dBV (so the true loopback signal is ~ -55 dBFS while an inserted
//! stimulus sample is 0.3 peak) — anything above 0.05 is an insertion.
use qa40x_driver::{InputGain, OutputGain, QA40xDevice, SampleRate};

#[tokio::main]
async fn main() {
    let dev = QA40xDevice::new();
    dev.connect().await.unwrap();
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    dev.set_input_gain(InputGain::Gain42dBV).await.unwrap();
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    for &n in &[8192usize, 32768, 131072] {
        let left: Vec<f32> = (0..n)
            .map(|i| 0.3 * (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin())
            .collect();
        let right: Vec<f32> = (0..n)
            .map(|i| 0.3 * (std::f32::consts::TAU * 1500.0 * i as f32 / 48000.0).sin())
            .collect();
        let calls = (10 * 32768 / n).clamp(3, 20);
        let mut bad_calls = 0;
        let mut total_bad = 0usize;
        let mut runs: Vec<String> = Vec::new();
        for c in 0..calls {
            let a = dev.generate_and_capture(&left, &right).await.unwrap();
            let bad: Vec<usize> = a
                .left_channel
                .iter()
                .enumerate()
                .filter(|(_, v)| v.abs() > 0.05)
                .map(|(i, _)| i)
                .collect();
            if !bad.is_empty() {
                bad_calls += 1;
                total_bad += bad.len();
                // group into runs
                let mut r = vec![(bad[0], bad[0])];
                for &i in &bad[1..] {
                    if i > r.last().unwrap().1 + 64 {
                        r.push((i, i));
                    } else {
                        r.last_mut().unwrap().1 = i;
                    }
                }
                runs.push(format!(
                    "call {c}: {}",
                    r.iter()
                        .map(|(a, b)| format!("{a}..{b}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
        println!(
            "N={n:>6}: {bad_calls}/{calls} calls with insertions, {total_bad} samples; {}",
            runs.join(" | ")
        );
    }
    dev.disconnect().await.unwrap();
}
