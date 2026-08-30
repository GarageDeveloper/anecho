//! Hardware diagnostic: drive one output at a time and report which input receives it.
use qa40x_driver::{Channel, InputGain, OutputGain, QA40xDevice, SampleRate};
#[tokio::main]
async fn main() {
    let dev = QA40xDevice::new();
    dev.connect().await.unwrap();
    let meta = dev.device_meta().await;
    println!(
        "unit: {:?}",
        meta.map(|m| format!("{:?}", m))
            .unwrap_or_default()
            .replace(|c: char| c.is_ascii_hexdigit() || c == '_', "*")
    );
    dev.set_sample_rate(SampleRate::Rate48kHz).await.unwrap();
    dev.set_input_gain(InputGain::Gain6dBV).await.unwrap();
    dev.set_output_gain(OutputGain::GainMinus2dBV)
        .await
        .unwrap();
    let n = 16384;
    let sine: Vec<f32> = (0..n)
        .map(|i| 0.3 * (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin())
        .collect();
    let zero = vec![0f32; n];
    let (in_l, _) = dev.input_dbv_offset(Channel::Left).await;
    let (in_r, _) = dev.input_dbv_offset(Channel::Right).await;
    let rms = |v: &[f32]| {
        20.0 * (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32)
            .sqrt()
            .log10()
    };
    for (label, l, r) in [
        ("drive LEFT only", &sine, &zero),
        ("drive RIGHT only", &zero, &sine),
        ("drive both", &sine, &sine),
    ] {
        let _ = dev.generate_and_capture(l, r).await.unwrap(); // settle
        let a = dev.generate_and_capture(l, r).await.unwrap();
        println!(
            "{label:<16}: input L {:7.2} dBV   input R {:7.2} dBV",
            rms(&a.left_channel[4096..]) + in_l,
            rms(&a.right_channel[4096..]) + in_r
        );
    }
    dev.disconnect().await.unwrap();
}
