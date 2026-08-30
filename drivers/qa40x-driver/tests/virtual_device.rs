//! End-to-end exercise of the embedded virtual QA40x (`sim` feature): connect,
//! identity, register bus, a real-time-paced generate-and-capture through the
//! simulated loopback, calibrated dBV levels, keepalive during a long capture,
//! then detach/reattach. No hardware, no USB.

use std::sync::Arc;
use std::time::{Duration, Instant};

use qa40x_driver::calibration_page_crc_ok;
use qa40x_driver::register::{RegisterOps, registers};
use qa40x_driver::{Channel, InputGain, OutputGain, QA40xDevice};

/// The embedded simulator is one per process (the single-attach guard): tests
/// in this binary must not attach concurrently.
static SIM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A sine of `amplitude` peak at `freq` Hz, `n` samples at `sample_rate`.
fn sine(freq: f32, amplitude: f32, sample_rate: u32, n: usize) -> Vec<f32> {
    let w = 2.0 * std::f32::consts::PI * freq / sample_rate as f32;
    (0..n).map(|i| amplitude * (w * i as f32).sin()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn virtual_demo_device_connect_capture_reconnect() {
    let _sim = SIM_LOCK.lock().await;
    let device = QA40xDevice::new();
    device.connect_virtual().await.expect("virtual connect");

    let meta = device.device_meta().await.expect("meta after connect");
    assert!(meta.is_virtual);
    assert_eq!(meta.model, "QA403");
    assert!(
        !meta.supports_flash,
        "the demo device must never offer a firmware flash"
    );
    assert_eq!(meta.sample_rates.last().copied(), Some(384_000));
    assert!(device.is_present().await);
    assert!(device.check_physical_connection().await);

    // Telemetry rides the same register bus as hardware.
    let t = device.read_telemetry().await.expect("telemetry");
    assert!(
        t.usb_voltage_v > 4.0 && t.usb_voltage_v < 6.0,
        "USB voltage {} V",
        t.usb_voltage_v
    );

    // The register bus is readable directly.
    let fw = device
        .read_register(registers::FIRMWARE_VERSION)
        .await
        .expect("firmware register");
    assert_eq!(fw.len(), 4);

    // The simulator serves a real factory calibration page: it must pass the
    // CRC check the official application applies.
    let page = device
        .read_calibration_page()
        .await
        .expect("calibration page");
    assert!(calibration_page_crc_ok(&page), "factory page CRC");

    // A tone through the simulated DAC→ADC loopback comes back at the level
    // the range/calibration model predicts. At out 8 dBV / in 18 dBV the
    // digital gain is outFS − inFS + 9 − trims ≈ −9.5 dB, so a 0.5-peak sine
    // captures at ≈ 0.17 — the bounds stay loose against trim details.
    device.set_input_gain(InputGain::Gain18dBV).await.unwrap();
    device.set_output_gain(OutputGain::Gain8dBV).await.unwrap();
    let tone = sine(1000.0, 0.5, 48_000, 4_800);
    let captured = device
        .generate_and_capture(&tone, &tone)
        .await
        .expect("generate_and_capture through the virtual loopback");
    assert_eq!(captured.sample_rate, 48_000);
    let peak = |ch: &[f32]| ch.iter().fold(0f32, |m, s| m.max(s.abs()));
    let (l, r) = (peak(&captured.left_channel), peak(&captured.right_channel));
    assert!(l > 0.05 && l < 0.5, "left loopback peak {l}");
    assert!(r > 0.05 && r < 0.5, "right loopback peak {r}");

    device.disconnect().await.expect("disconnect");
    assert!(!device.is_connected().await);
    assert!(!device.is_virtual(), "detached from the simulator");
    // Presence now only reflects real hardware on the bus (bench-dependent).
    assert_eq!(
        device.is_present().await,
        device.is_hardware_present().await
    );

    // The single-attach guard must release on disconnect: a second demo
    // session (same simulator, state kept) attaches cleanly.
    device.connect_virtual().await.expect("virtual reconnect");
    assert!(device.is_connected().await);
    device.disconnect().await.expect("second disconnect");
}

/// A pre-armed cancel flag must abort the very first block of a capture
/// rather than being checked too late (or not at all).
#[tokio::test(flavor = "multi_thread")]
async fn generate_and_capture_honors_a_preset_cancel_flag() {
    let _sim = SIM_LOCK.lock().await;
    let device = QA40xDevice::new();
    device.connect_virtual().await.expect("virtual connect");

    let cancel = std::sync::atomic::AtomicBool::new(true);
    let tone = sine(1000.0, 0.2, 48_000, 96_000);
    let err = device
        .generate_and_capture_cancellable(&tone, &tone, Some(&cancel))
        .await
        .expect_err("a pre-armed cancel flag must abort the capture");
    assert!(
        matches!(err, qa40x_driver::QA40xError::Cancelled),
        "unexpected error: {err:?}"
    );

    device.disconnect().await.expect("disconnect");
}

/// A dBV-denominated stimulus pre-compensated by the per-unit DAC trims must
/// come back — through the simulator's calibrated DAC→loopback→ADC chain and
/// the ADC-calibrated readout — at exactly the commanded level. Without the
/// trims the +8 dBV range reads a few tenths of a dB hot, so the 0.1 dB
/// bound fails.
#[tokio::test(flavor = "multi_thread")]
async fn dbv_stimulus_lands_at_the_commanded_level_once_trimmed() {
    let _sim = SIM_LOCK.lock().await;
    let device = QA40xDevice::new();
    device.connect_virtual().await.expect("virtual connect");
    device.set_input_gain(InputGain::Gain6dBV).await.unwrap();
    device.set_output_gain(OutputGain::Gain8dBV).await.unwrap();

    let (trims, calibrated) = device.dac_trims().await;
    assert!(
        calibrated,
        "the simulator serves a real factory calibration page"
    );

    // −10 dBV on the +8 dBV range: ideal digital amplitude 10^(−18/20),
    // then the per-channel trim.
    let sr = 48_000u32;
    let n = 4_800usize;
    let ideal = 10f32.powf((-10.0 - 8.0) / 20.0);
    let left = sine(1000.0, ideal * trims.0, sr, n);
    let right = sine(1000.0, ideal * trims.1, sr, n);
    let captured = device
        .generate_and_capture(&left, &right)
        .await
        .expect("loopback capture");

    // RMS over the LAST 70 % — an integer 70 cycles of 1 kHz, clear of the
    // simulator's loopback latency — converted to dBV through the ADC
    // calibration.
    let level_dbv = |sig: &[f32], offset_db: f32| -> f32 {
        let tail = &sig[3 * n / 10..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * rms.log10() + offset_db
    };
    let (off_l, cal_l) = device.input_dbv_offset(Channel::Left).await;
    let (off_r, _) = device.input_dbv_offset(Channel::Right).await;
    assert!(cal_l, "ADC side reads the same calibration page");
    let l = level_dbv(&captured.left_channel, off_l);
    let r = level_dbv(&captured.right_channel, off_r);
    assert!(
        (l + 10.0).abs() < 0.1,
        "left loopback level {l} dBV, commanded -10"
    );
    assert!(
        (r + 10.0).abs() < 0.1,
        "right loopback level {r} dBV, commanded -10"
    );

    device.disconnect().await.expect("disconnect");
}

/// During a capture the stream pump itself must fire the LINK keepalive at
/// ~1 Hz, not just once before the stream starts — otherwise a long capture
/// leaves the LINK LED to time out mid-run. The simulator is real-time-paced,
/// so a ~3 s capture is long enough for more than one in-capture keepalive.
#[tokio::test(flavor = "multi_thread")]
async fn in_capture_keepalive_fires_at_roughly_1hz_during_a_long_capture() {
    let _sim = SIM_LOCK.lock().await;
    let device = Arc::new(QA40xDevice::new());
    device.connect_virtual().await.expect("virtual connect");

    assert!(device.last_keepalive_at().await.is_none());
    assert!(device.last_telemetry().await.is_none());

    // Baseline keepalive: consumes the rate-limit slot right before the
    // capture, so every later stamp is attributable to the in-capture path.
    device.keepalive().await.expect("baseline keepalive");
    let baseline = device
        .last_keepalive_at()
        .await
        .expect("baseline stamp recorded");
    assert!(device.last_telemetry().await.is_some());

    let sr = 48_000u32;
    let n = 150_000usize; // ~3.1 s of real-time-paced capture
    let tone = sine(1000.0, 0.2, sr, n);

    // Watcher: polls `last_keepalive_at()` — a mutex read with no device I/O.
    let watcher_dev = device.clone();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let watcher = tokio::spawn(async move {
        let mut stamps: Vec<Instant> = Vec::new();
        loop {
            if let Some(t) = watcher_dev.last_keepalive_at().await
                && stamps.last() != Some(&t)
            {
                stamps.push(t);
            }
            tokio::select! {
                _ = &mut stop_rx => break,
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }
        stamps
    });

    let captured = device
        .generate_and_capture(&tone, &tone)
        .await
        .expect("generate_and_capture through the virtual loopback");

    let _ = stop_tx.send(());
    let stamps = watcher.await.expect("watcher task");

    assert_eq!(captured.sample_rate, sr);
    assert_eq!(captured.left_channel.len(), n, "left channel truncated");
    assert_eq!(captured.right_channel.len(), n, "right channel truncated");

    let in_capture = stamps.iter().filter(|t| **t > baseline).count();
    assert!(
        in_capture >= 2,
        "expected the pump to fire the in-capture keepalive at least twice \
         during a ~3 s capture (~1 Hz), got {in_capture} stamps after baseline: {stamps:?}"
    );

    let after = device
        .last_keepalive_at()
        .await
        .expect("stamp after capture");
    assert!(
        after > baseline,
        "last_keepalive_at did not advance past the pre-capture baseline"
    );
    assert!(device.last_telemetry().await.is_some());

    device.disconnect().await.expect("disconnect");
}

/// The discovery layer enumerates the virtual units next to the USB bus and
/// opens one by id onto a shared handle.
#[tokio::test(flavor = "multi_thread")]
async fn virtual_source_enumerates_and_opens_by_id() {
    use qa40x_driver::{DeviceHandle, DeviceSource, VirtualDeviceSource};

    let _sim = SIM_LOCK.lock().await;
    let src = VirtualDeviceSource::builtin();
    let descs = src.enumerate().await.expect("virtual enumerate");
    assert!(!descs.is_empty());
    assert!(descs.iter().all(|d| d.identity.is_virtual));

    let handle: DeviceHandle = Arc::new(tokio::sync::Mutex::new(QA40xDevice::new()));
    let opened = src.open(&descs[0].id, &handle).await.expect("open unit 0");
    assert_eq!(opened.id, descs[0].id);
    assert!(handle.lock().await.is_connected().await);
    handle.lock().await.disconnect().await.expect("disconnect");
}
