//! Phase 1 / task 4: RTA and SCOPE streams through the API only.

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

fn sine(hz: f32, peak_dbfs: f32) -> pb::Generator {
    pb::Generator {
        signal: Some(pb::generator::Signal::Sine(pb::generator::Sine {
            frequency_hz: hz,
            amplitude_dbfs: peak_dbfs,
        })),
        ..Default::default()
    }
}

async fn nth_frame(
    frames: &mut tokio::sync::broadcast::Receiver<anecho_wire::Frame>,
    stream_id: u32,
    n: usize,
) -> anecho_wire::Frame {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut seen = 0;
    loop {
        let f = tokio::time::timeout_at(deadline, frames.recv())
            .await
            .expect("frames keep coming")
            .unwrap();
        if f.stream_id == stream_id {
            seen += 1;
            if seen == n {
                return f;
            }
        }
    }
}

fn nearest(axis: &[f32], hz: f32) -> usize {
    axis.iter()
        .enumerate()
        .min_by(|a, b| (a.1 - hz).abs().partial_cmp(&(b.1 - hz).abs()).unwrap())
        .unwrap()
        .0
}

async fn virtual_session(url: &str) -> (Client, u64) {
    let client = Client::connect(url).await.unwrap();
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
    (client, session.session_id)
}

#[tokio::test]
async fn rta_log_axis_reads_the_sine_level_and_a_clean_floor() {
    let registry = DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::new(
        LoopbackOptions {
            realtime: true,
            ..Default::default()
        },
    )));
    let (url, _stop) = serve(registry).await;
    let (client, session_id) = virtual_session(&url).await;
    let mut frames = client.frames();
    // 1500 Hz sits exactly on bin 512 of a 16384-point FFT at 48 kHz.
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id,
            kind: pb::StreamKind::Rta as i32,
            block_frames: 4096,
            generator: Some(sine(1500.0, -20.0)),
            rta: Some(pb::RtaConfig {
                fft_length: 16_384,
                window: pb::rta_config::Window::Hann as i32,
                points: 1000,
                min_hz: 20.0,
                max_hz: 20_000.0,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(stream.kind, pb::StreamKind::Rta as i32);
    assert_eq!(stream.axis_hz.len(), 1000);
    assert_eq!(stream.values_per_channel, 1000);
    // Skip the first frame (it may include the loopback latency silence).
    let f = nth_frame(&mut frames, stream.stream_id, 2).await;
    let i = nearest(&stream.axis_hz, 1500.0);
    let v = f.channel(0)[i];
    // −20 dBFS peak → −23.01 dBFS RMS (LEVELS convention).
    assert!(
        (v + 23.01).abs() < 0.5,
        "{v} dBFS at {} Hz",
        stream.axis_hz[i]
    );
    for hz in [150.0, 15_000.0] {
        let j = nearest(&stream.axis_hz, hz);
        assert!(f.channel(0)[j] < -80.0, "{} dB at {hz} Hz", f.channel(0)[j]);
    }
    client.stop_stream(stream.stream_id).await.unwrap();
}

#[tokio::test]
async fn rta_third_octave_bands() {
    let registry = DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::new(
        LoopbackOptions {
            realtime: true,
            ..Default::default()
        },
    )));
    let (url, _stop) = serve(registry).await;
    let (client, session_id) = virtual_session(&url).await;
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id,
            kind: pb::StreamKind::Rta as i32,
            block_frames: 4096,
            generator: Some(sine(1500.0, -20.0)),
            rta: Some(pb::RtaConfig {
                fft_length: 16_384,
                octave_fraction: 3,
                min_hz: 20.0,
                max_hz: 20_000.0,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    let f = nth_frame(&mut frames, stream.stream_id, 2).await;
    let band = nearest(&stream.axis_hz, 1587.4);
    assert!(
        (f.channel(0)[band] + 23.01).abs() < 0.3,
        "{}",
        f.channel(0)[band]
    );
    for (j, v) in f.channel(0).iter().enumerate() {
        if j != band {
            assert!(*v < -80.0, "band {} Hz: {v}", stream.axis_hz[j]);
        }
    }
    client.stop_stream(stream.stream_id).await.unwrap();
}

#[tokio::test]
async fn scope_rising_trigger_starts_at_the_zero_crossing() {
    let registry = DeviceRegistry::new().with_backend(Arc::new(VirtualLoopbackBackend::new(
        LoopbackOptions {
            realtime: true,
            ..Default::default()
        },
    )));
    let (url, _stop) = serve(registry).await;
    let (client, session_id) = virtual_session(&url).await;
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id,
            kind: pb::StreamKind::Scope as i32,
            block_frames: 480,
            generator: Some(sine(1000.0, -6.0)),
            scope: Some(pb::ScopeConfig {
                window_frames: 480,
                points: 480,
                trigger: Some(pb::scope_config::Trigger {
                    mode: pb::scope_config::trigger::Mode::Rising as i32,
                    level: 0.0,
                    channel: 0,
                }),
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(stream.axis_seconds.len(), 480);
    assert!((stream.axis_seconds[1] - 1.0 / 48_000.0).abs() < 1e-9);
    let f = nth_frame(&mut frames, stream.stream_id, 6).await;
    let x = f.channel(0);
    // Rising crossing: at or just above zero, then increasing; peak ≈ 0.5.
    assert!(x[0] >= 0.0 && x[0] < 0.08, "x0 {}", x[0]);
    assert!(x[1] > x[0] && x[12] > 0.45, "x1 {} x12 {}", x[1], x[12]);
    client.stop_stream(stream.stream_id).await.unwrap();
}

/// RTA on the simulated QA40x: values arrive in dBV through the factory calibration.
#[cfg(feature = "qa40x-sim")]
#[tokio::test]
async fn rta_on_the_simulated_qa40x_reads_dbv() {
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
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            kind: pb::StreamKind::Rta as i32,
            block_frames: 4096,
            generator: Some(pb::Generator {
                level: Some(pb::generator::Level {
                    unit: Some(pb::generator::level::Unit::DbvRms(-20.0)),
                }),
                ..sine(1500.0, 0.0)
            }),
            rta: Some(pb::RtaConfig {
                fft_length: 16_384,
                points: 500,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(
        stream.scale.as_ref().and_then(|s| s.unit.as_ref()),
        Some(pb::scale::Unit::DbvOffset(_))
    ));
    let f = nth_frame(&mut frames, stream.stream_id, 3).await;
    let i = nearest(&stream.axis_hz, 1500.0);
    let v = f.channel(0)[i];
    assert!((v + 20.0).abs() < 0.5, "{v} dBV");
    client.stop_stream(stream.stream_id).await.unwrap();
}
