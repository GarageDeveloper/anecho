//! Hardware diagnostic: once the device is in the "DAC data in ADC stream" state, does
//! any software action clear it? Each candidate is followed by a raw insertion probe.
use qa40x_driver::{InputGain, OutputGain, QA40xDevice, SampleRate};
use std::time::Duration;

async fn probe(dev: &QA40xDevice, label: &str) -> usize {
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
    println!("{label:<52} {bad}/20");
    bad
}

#[tokio::main]
async fn main() {
    let dev = QA40xDevice::new();
    dev.connect().await.unwrap();
    println!("model {:?}", dev.model().await);
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    dev.set_input_gain(InputGain::Gain42dBV).await.unwrap();
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    if probe(&dev, "state on entry").await == 0 {
        println!("device is clean, nothing to recover");
        return;
    }

    // r1: idle input-range writes with long pauses
    for g in [InputGain::Gain6dBV, InputGain::Gain42dBV] {
        dev.set_input_gain(g).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    probe(&dev, "r1 idle input-range writes (500 ms pauses)").await;
    // r2: sample-rate rewrite
    dev.set_sample_rate(SampleRate::Rate96kHz).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    probe(&dev, "r2 sample rate 96k -> 48k").await;
    // r3: a long silent capture (drains FIFOs?)
    let _ = dev.acquire_data(131072).await.unwrap();
    probe(&dev, "r3 long silent acquire (131072)").await;
    // r4: software disconnect / reconnect (USB re-claim, no power cycle)
    dev.disconnect().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    dev.connect().await.unwrap();
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    dev.set_input_gain(InputGain::Gain42dBV).await.unwrap();
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    probe(&dev, "r4 software disconnect/reconnect").await;
    // r5: output-range write while idle
    dev.set_output_gain(OutputGain::GainMinus12dBV)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    probe(&dev, "r5 idle output-range writes").await;
    dev.disconnect().await.unwrap();
}
