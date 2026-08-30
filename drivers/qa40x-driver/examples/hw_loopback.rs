//! Hardware loopback check: connect to the first QA40x on the bus, dump and
//! verify the factory calibration page, then play a 1 kHz tone through a
//! resistive loopback and read the captured level back in absolute dBV.
//!
//! Wiring: OUT L+ -> IN L+, OUT R+ -> IN R+, IN L-/R- terminated.
//! Run with: cargo run -p qa40x-driver --example hw_loopback

use qa40x_driver::{
    CalibrationData, Channel, InputGain, OutputGain, QA40xDevice, SampleRate,
    calibration_page_crc_ok,
};

fn sine(freq: f32, amplitude: f32, sample_rate: u32, n: usize) -> Vec<f32> {
    let w = 2.0 * std::f32::consts::PI * freq / sample_rate as f32;
    (0..n).map(|i| amplitude * (w * i as f32).sin()).collect()
}

fn rms_dbfs(sig: &[f32]) -> f32 {
    let tail = &sig[sig.len() / 4..];
    let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
    20.0 * rms.log10()
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let device = QA40xDevice::new();
    device
        .connect()
        .await
        .expect("no QA40x found on the USB bus");
    let meta = device.device_meta().await.expect("device meta");
    println!(
        "Connected: {} fw {} ({})",
        meta.model, meta.firmware_version, meta.product
    );

    println!("\n== Factory calibration page ==");
    let page = device
        .read_calibration_page()
        .await
        .expect("calibration page");
    println!(
        "  {} bytes, CRC {}",
        page.len(),
        if calibration_page_crc_ok(&page) {
            "ok"
        } else {
            "MISMATCH"
        }
    );
    for dbv in [0, 6, 12, 18, 24, 30, 36, 42] {
        if let Some(off) = CalibrationData::adc_offset(dbv)
            && off + 12 <= page.len()
        {
            let rec =
                |o: usize| f32::from_le_bytes([page[o + 2], page[o + 3], page[o + 4], page[o + 5]]);
            println!(
                "  ADC {dbv:>2} dBV: L {:+.3} dB  R {:+.3} dB",
                rec(off),
                rec(off + 6)
            );
        }
    }
    for dbv in [-12, -2, 8, 18] {
        if let Some(off) = CalibrationData::dac_offset(dbv)
            && off + 12 <= page.len()
        {
            let rec =
                |o: usize| f32::from_le_bytes([page[o + 2], page[o + 3], page[o + 4], page[o + 5]]);
            println!(
                "  DAC {dbv:>3} dBV: L {:+.3} dB  R {:+.3} dB",
                rec(off),
                rec(off + 6)
            );
        }
    }

    println!("\n== Loopback tone: 1 kHz, -10 dBV commanded, out +8 dBV / in +6 dBV ==");
    device
        .set_sample_rate(SampleRate::Rate48kHz)
        .await
        .expect("sample rate");
    device
        .set_output_gain(OutputGain::Gain8dBV)
        .await
        .expect("output range");
    device
        .set_input_gain(InputGain::Gain6dBV)
        .await
        .expect("input range");

    let (trims, trimmed) = device.dac_trims().await;
    let ideal = 10f32.powf((-10.0 - 8.0) / 20.0);
    let n = 48_000;
    let left = sine(1000.0, ideal * trims.0, 48_000, n);
    let right = sine(1000.0, ideal * trims.1, 48_000, n);
    let captured = device
        .generate_and_capture(&left, &right)
        .await
        .expect("generate_and_capture");

    for (ch, sig) in [
        (Channel::Left, &captured.left_channel),
        (Channel::Right, &captured.right_channel),
    ] {
        let (offset, calibrated) = device.input_dbv_offset(ch).await;
        let dbfs = rms_dbfs(sig);
        println!(
            "  {ch:?}: {dbfs:+.2} dBFS -> {:+.2} dBV (ADC cal {}, DAC trims {})",
            dbfs + offset,
            if calibrated { "applied" } else { "nominal" },
            if trimmed { "applied" } else { "nominal" },
        );
    }

    device.disconnect().await.expect("disconnect");
    println!("\nDone.");
}
