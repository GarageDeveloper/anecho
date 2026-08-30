//! Phase 1 / task 5: one-shot distortion measurements and input auto-range, API only.

use anecho_client::Client;
use anecho_contract::v0 as pb;
use anecho_device::DeviceRegistry;
use anecho_device::backends::virtual_loopback::{LoopbackOptions, VirtualLoopbackBackend};
use anecho_engine::Engine;
use std::sync::Arc;
#[cfg(feature = "qa40x-sim")]
use std::time::Duration;

async fn serve(registry: DeviceRegistry) -> (String, tokio::sync::oneshot::Sender<()>) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (addr, _task) = anecho_server::serve(
        Engine::new(registry),
        "127.0.0.1:0".parse().unwrap(),
        async {
            let _ = stop_rx.await;
        },
    )
    .await
    .unwrap();
    (format!("ws://{addr}/ws"), stop_tx)
}

fn tone(signal: pb::generator::Signal, level: pb::generator::level::Unit) -> pb::Generator {
    pb::Generator {
        signal: Some(signal),
        level: Some(pb::generator::Level { unit: Some(level) }),
        ..Default::default()
    }
}

async fn virtual_backend() -> (String, tokio::sync::oneshot::Sender<()>) {
    serve(
        DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::new(
            LoopbackOptions {
                // −120 dBFS-ish white noise so THD+N is finite and the floor is realistic.
                noise_peak: 2e-6,
                ..Default::default()
            },
        ))),
    )
    .await
}

#[tokio::test]
async fn thd_of_a_clean_sine_through_the_virtual_loopback() {
    let (url, _stop) = virtual_backend().await;
    let client = Client::connect(&url).await.unwrap();
    let dev = client.list_devices().await.unwrap().remove(0);
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m = client
        .measure(pb::MeasureRequest {
            session_id: session.session_id,
            kind: pb::MeasureKind::Thd as i32,
            generator: Some(tone(
                pb::generator::Signal::Sine(pb::generator::Sine {
                    frequency_hz: 1000.0,
                    amplitude_dbfs: 0.0,
                }),
                pb::generator::level::Unit::PeakDbfs(-20.0),
            )),
            fft_length: 32_768,
            averages: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(m.kind, pb::MeasureKind::Thd as i32);
    assert_eq!(m.per_channel.len(), 2);
    let r = &m.per_channel[0];
    assert!(
        (r.fundamental_hz - 1000.0).abs() < 1.0,
        "{}",
        r.fundamental_hz
    );
    // −20 dBFS peak → −23.01 dBFS RMS in the session scale.
    assert!(
        (r.fundamental_level + 23.01).abs() < 0.1,
        "{}",
        r.fundamental_level
    );
    assert!(r.thd_pct < 0.001, "thd {}", r.thd_pct);
    assert!(r.thd_n_pct < 0.01, "thd+n {}", r.thd_n_pct);
    assert_eq!(r.harmonics.len(), 8);
    assert!(r.harmonics.iter().all(|h| h.level_db_rel < -100.0));

    // A second measurement on the same session works (the device was released).
    let again = client
        .measure(pb::MeasureRequest {
            session_id: session.session_id,
            kind: pb::MeasureKind::Thd as i32,
            generator: Some(tone(
                pb::generator::Signal::Sine(pb::generator::Sine {
                    frequency_hz: 2000.0,
                    amplitude_dbfs: 0.0,
                }),
                pb::generator::level::Unit::PeakDbfs(-20.0),
            )),
            fft_length: 16_384,
            averages: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!((again.per_channel[1].fundamental_hz - 2000.0).abs() < 2.0);
    client.close_session(session.session_id).await.unwrap();
}

#[tokio::test]
async fn imd_smpte_of_a_clean_dual_tone() {
    let (url, _stop) = virtual_backend().await;
    let client = Client::connect(&url).await.unwrap();
    let dev = client.list_devices().await.unwrap().remove(0);
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m = client
        .measure(pb::MeasureRequest {
            session_id: session.session_id,
            kind: pb::MeasureKind::ImdSmpte as i32,
            generator: Some(tone(
                pb::generator::Signal::DualTone(pb::generator::DualTone {
                    f1_hz: 60.0,
                    f2_hz: 7000.0,
                    ratio_db: 12.04,
                }),
                pb::generator::level::Unit::PeakDbfs(-6.0),
            )),
            fft_length: 32_768,
            averages: 1,
            ..Default::default()
        })
        .await
        .unwrap();
    let r = &m.per_channel[0];
    assert!(r.imd_pct < 0.01, "imd {} %", r.imd_pct);
    assert!(r.imd_db < -80.0);
    client.close_session(session.session_id).await.unwrap();
}

#[tokio::test]
async fn measure_is_refused_while_a_stream_runs() {
    let (url, _stop) = virtual_backend().await;
    let client = Client::connect(&url).await.unwrap();
    let dev = client.list_devices().await.unwrap().remove(0);
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            kind: pb::StreamKind::Levels as i32,
            ..Default::default()
        })
        .await
        .unwrap();
    let err = client
        .measure(pb::MeasureRequest {
            session_id: session.session_id,
            kind: pb::MeasureKind::Thd as i32,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        anecho_client::ClientError::Server {
            code: pb::ErrorCode::Busy,
            ..
        }
    ));
    client.stop_stream(stream.stream_id).await.unwrap();
}

/// THD on the simulated QA40x (its model injects H2 at −90 dBc and H3 at −100 dBc).
#[cfg(feature = "qa40x-sim")]
#[tokio::test]
async fn thd_on_the_simulated_qa40x() {
    use anecho_device::backends::qa40x::Qa40xBackend;
    let (url, _stop) = serve(
        DeviceRegistry::new().with_backend(Arc::new(Qa40xBackend::empty().with_simulator(false))),
    )
    .await;
    let client = Client::connect(&url).await.unwrap();
    let dev = client.list_devices().await.unwrap().remove(0);
    let in_range = dev
        .input_ranges
        .iter()
        .position(|r| r.full_scale_dbv == 6.0)
        .unwrap() as u32;
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                input_range: Some(in_range),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let m = client
        .measure(pb::MeasureRequest {
            session_id: session.session_id,
            kind: pb::MeasureKind::Thd as i32,
            generator: Some(tone(
                pb::generator::Signal::Sine(pb::generator::Sine {
                    frequency_hz: 1000.0,
                    amplitude_dbfs: 0.0,
                }),
                pb::generator::level::Unit::DbvRms(-10.0),
            )),
            fft_length: 32_768,
            averages: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(
        m.scale.as_ref().and_then(|s| s.unit.as_ref()),
        Some(pb::scale::Unit::DbvOffset(_))
    ));
    let Some(pb::scale::Unit::DbvOffset(in_off)) = m.scale.as_ref().and_then(|s| s.unit) else {
        unreachable!()
    };
    let r = &m.per_channel[0];
    assert!((r.fundamental_hz - 1000.0).abs() < 1.0);
    assert!(
        (r.fundamental_level + 10.0).abs() < 0.5,
        "{} dBV",
        r.fundamental_level
    );
    // The simulator's H2 is a quadratic term: −90 dBc at full scale, so at a fundamental
    // of `p` dBFS peak it sits at −90 + p dBc.
    let peak_dbfs = r.fundamental_level - in_off + 3.01;
    let h2 = r.harmonics.iter().find(|h| h.order == 2).unwrap();
    let expected_h2 = -90.0 + peak_dbfs;
    assert!(
        (h2.level_db_rel - expected_h2).abs() < 3.0,
        "H2 {} dBc, expected {expected_h2} (fundamental {peak_dbfs} dBFS peak)",
        h2.level_db_rel
    );
    // THD is dominated by H2.
    let expected_thd = 100.0 * 10f32.powf(expected_h2 / 20.0);
    assert!(
        (r.thd_pct / expected_thd - 1.0).abs() < 0.5,
        "thd {} % vs {expected_thd}",
        r.thd_pct
    );
    client.close_session(session.session_id).await.unwrap();
}

/// Input auto-range on the simulated QA40x: from the 42 dBV range down to the range that
/// fits a −20 dBV sine, with the level meter unchanged in dBV across every switch.
#[cfg(feature = "qa40x-sim")]
#[tokio::test]
async fn input_auto_range_steps_down_and_keeps_dbv_readings() {
    use anecho_device::backends::qa40x::Qa40xBackend;
    let (url, _stop) = serve(
        DeviceRegistry::new().with_backend(Arc::new(Qa40xBackend::empty().with_simulator(false))),
    )
    .await;
    let client = Client::connect(&url).await.unwrap();
    let dev = client.list_devices().await.unwrap().remove(0);
    let top = dev.input_ranges.len() as u32 - 1;
    assert_eq!(dev.input_ranges[top as usize].full_scale_dbv, 42.0);
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                input_range: Some(top),
                auto_range_input: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut events = client.events();
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            kind: pb::StreamKind::Levels as i32,
            block_frames: 4096,
            levels_rate_hz: 10.0,
            generator: Some(tone(
                pb::generator::Signal::Sine(pb::generator::Sine {
                    frequency_hz: 1000.0,
                    amplitude_dbfs: 0.0,
                }),
                pb::generator::level::Unit::DbvRms(-20.0),
            )),
            ..Default::default()
        })
        .await
        .unwrap();

    // Collect range changes until the range stops moving (≤ 30 s of simulated audio).
    let mut ranges = Vec::new();
    let mut readings: Vec<f32> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        tokio::select! {
            ev = events.recv() => {
                if let Ok(pb::Event { kind: Some(pb::event::Kind::RangeChanged(r)) }) = ev
                    && let Some(i) = r.input_range
                {
                    ranges.push(i);
                }
            }
            f = frames.recv() => {
                if let Ok(f) = f && f.stream_id == stream.stream_id {
                    readings.push(f.channel(0)[0]);
                }
            }
            _ = tokio::time::sleep_until(deadline) => panic!("auto-range never settled"),
        }
        // −20 dBV RMS: at 0 dBV the peak is −20+3−3.6 ≈ −20.6 dBFS (< −18, still "low"),
        // so the policy goes all the way down to range 0.
        if ranges.last() == Some(&0) && readings.len() >= 20 {
            break;
        }
    }
    // Strictly descending, one step at a time.
    assert_eq!(ranges[0], top - 1, "{ranges:?}");
    assert!(ranges.windows(2).all(|w| w[1] + 1 == w[0]), "{ranges:?}");
    assert_eq!(*ranges.last().unwrap(), 0);
    // Readings stay at −20 dBV (±0.5) whatever the range, apart from the block during
    // which the relay switched (allow a few outliers).
    let outliers = readings.iter().filter(|v| (**v + 20.0).abs() > 0.5).count();
    assert!(
        outliers <= ranges.len() + 2,
        "{outliers} outliers in {readings:?}"
    );
    client.stop_stream(stream.stream_id).await.unwrap();
}
