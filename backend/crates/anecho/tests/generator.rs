//! Phase 1 / task 3: the full generator through the API only.

use anecho_client::Client;
use anecho_contract::v0 as pb;
use anecho_device::DeviceRegistry;
use anecho_device::backends::virtual_loopback::{LoopbackOptions, VirtualLoopbackBackend};
use anecho_engine::Engine;
use std::sync::Arc;
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

async fn collect(
    frames: &mut tokio::sync::broadcast::Receiver<anecho_wire::Frame>,
    stream_id: u32,
    n: usize,
) -> Vec<anecho_wire::Frame> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while out.len() < n {
        let f = tokio::time::timeout_at(deadline, frames.recv())
            .await
            .expect("frames keep coming")
            .unwrap();
        if f.stream_id == stream_id {
            out.push(f);
        }
    }
    out
}

/// dBFS level on the virtual loopback, one channel driven.
#[tokio::test]
async fn pink_noise_level_in_dbfs_on_the_selected_channel() {
    let registry = DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::new(
        LoopbackOptions {
            realtime: true,
            ..Default::default()
        },
    )));
    let (url, _stop) = serve(registry).await;
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
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            kind: pb::StreamKind::Levels as i32,
            block_frames: 4800,
            // Pink noise has real energy below 10 Hz: measure over 0.5 s windows.
            levels_rate_hz: 2.0,
            generator: Some(pb::Generator {
                signal: Some(pb::generator::Signal::Noise(pb::generator::Noise {
                    kind: pb::generator::NoiseKind::Pink as i32,
                    period_frames: 0,
                    seed: 7,
                })),
                level: Some(pb::generator::Level {
                    unit: Some(pb::generator::level::Unit::PeakDbfs(-20.0)),
                }),
                output_channels: vec![0],
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let readings = collect(&mut frames, stream.stream_id, 5).await;
    // Noise RMS = peak/√2 → −23.01 dBFS_rms on the driven channel (±1.5 dB over 0.5 s of
    // pink noise), silence on the other.
    for f in &readings[1..] {
        let l = f.channel(0)[0];
        let r = f.channel(1)[0];
        assert!((l + 23.01).abs() < 1.5, "left rms {l} dBFS");
        assert!(r < -150.0, "right must be silent, got {r} dBFS");
    }
    client.stop_stream(stream.stream_id).await.unwrap();
    client.close_session(session.session_id).await.unwrap();
}

/// A dBV level on a device without calibration is refused with a typed error.
#[tokio::test]
async fn dbv_level_is_unsupported_without_calibration() {
    let registry = DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::default()));
    let (url, _stop) = serve(registry).await;
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
    let err = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            kind: pb::StreamKind::Levels as i32,
            generator: Some(pb::Generator {
                signal: Some(pb::generator::Signal::Square(pb::generator::Square {
                    frequency_hz: 100.0,
                })),
                level: Some(pb::generator::Level {
                    unit: Some(pb::generator::level::Unit::DbvRms(0.0)),
                }),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        anecho_client::ClientError::Server {
            code: pb::ErrorCode::Unsupported,
            ..
        }
    ));
}

/// A dBV level closes the loop generator → output range → calibration → level meter on the
/// simulated QA40x, and the range change is announced.
#[cfg(feature = "qa40x-sim")]
#[tokio::test]
async fn sine_level_in_dbv_fits_the_output_range() {
    use anecho_device::backends::qa40x::Qa40xBackend;
    let registry =
        DeviceRegistry::new().with_backend(Arc::new(Qa40xBackend::empty().with_simulator(false)));
    let (url, _stop) = serve(registry).await;
    let client = Client::connect(&url).await.unwrap();
    let dev = client.list_devices().await.unwrap().remove(0);
    let in_range = dev
        .input_ranges
        .iter()
        .position(|r| r.full_scale_dbv == 6.0)
        .unwrap() as u32;
    let expected_out = dev
        .output_ranges
        .iter()
        .position(|r| r.full_scale_dbv == -2.0)
        .unwrap() as u32;
    // Open on the lowest output range (−12 dBV): −10 dBV RMS needs the −2 dBV range.
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                input_range: Some(in_range),
                output_range: Some(0),
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
            generator: Some(pb::Generator {
                signal: Some(pb::generator::Signal::Sine(pb::generator::Sine {
                    frequency_hz: 1000.0,
                    amplitude_dbfs: 0.0,
                })),
                level: Some(pb::generator::Level {
                    unit: Some(pb::generator::level::Unit::DbvRms(-10.0)),
                }),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let ev = tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("a RangeChanged event")
        .unwrap();
    match ev.kind {
        Some(pb::event::Kind::RangeChanged(r)) => {
            assert_eq!(r.session_id, session.session_id);
            assert_eq!(r.output_range, Some(expected_out));
            assert_eq!(r.input_range, None);
        }
        other => panic!("unexpected event {other:?}"),
    }
    let readings = collect(&mut frames, stream.stream_id, 6).await;
    for f in &readings[2..] {
        let v = f.channel(0)[0];
        assert!((v + 10.0).abs() < 0.5, "rms {v} dBV");
    }
    client.stop_stream(stream.stream_id).await.unwrap();
    client.close_session(session.session_id).await.unwrap();
}
