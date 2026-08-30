//! Hardware diagnostic: sweep every input-range transition of a QA40x wired in loopback
//! and flag those whose first capture is off by more than 0.5 dB. Used to characterise the
//! attenuator-out quirk worked around in `qa40x_driver::QA40xDevice::set_input_gain`
//! (2026-08-30: 0/56 failures with the workaround, 6–9/56 without).
//!
//! Needs a QA402/QA403 with outputs wired to inputs. Run with:
//! `cargo run -p anecho-device --example qa40x_relay_matrix`

use qa40x_driver::{Channel, InputGain, OutputGain, QA40xDevice, SampleRate};
use std::time::Duration;

#[tokio::main]
async fn main() {
    env_logger::init();
    let dev = QA40xDevice::new();
    dev.connect().await.unwrap();
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    let (out_off, _) = dev.output_dbv_offset(Channel::Left).await;
    let expected = out_off + 20.0 * (0.1f32 / 2f32.sqrt()).log10();
    let n = 8192;
    let sine: Vec<f32> = (0..n)
        .map(|i| 0.1 * (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin())
        .collect();
    let mut bad = Vec::new();
    for from in InputGain::ALL {
        for to in InputGain::ALL {
            if from == to {
                continue;
            }
            dev.set_input_gain(from).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            dev.set_input_gain(to).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
            let a = dev.generate_and_capture(&sine, &sine).await.unwrap();
            let tail = &a.left_channel[a.left_channel.len() / 2..];
            let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
            let (off, _) = dev.input_dbv_offset(Channel::Left).await;
            let dbv = 20.0 * rms.log10() + off;
            let err = dbv - expected;
            let mark = if err.abs() < 0.5 { "ok " } else { "BAD" };
            if err.abs() >= 0.5 {
                bad.push((from.as_dbv(), to.as_dbv(), err));
            }
            println!(
                "{mark} {:>2} -> {:>2} dBV : {dbv:7.2} dBV (err {err:+6.2})",
                from.as_dbv(),
                to.as_dbv()
            );
        }
    }
    println!("expected {expected:.2} dBV; failing transitions: {bad:?}");
    dev.set_input_gain(InputGain::Gain42dBV).await.unwrap();
    dev.disconnect().await.unwrap();
}
