//! The persistent capture pipeline: switching streams (kind, FFT, generator) on a session
//! must reuse the device's running loop — exactly ONE device `start()` — and never cancel
//! a fresh capture (the sequence that corrupts a QA402, see the qa40x backend docs).

#![cfg(feature = "qa40x-sim")]

use anecho_device::backends::qa40x::Qa40xBackend;
use anecho_device::{
    AppliedConfig, BackendKind, DeviceBackend, DeviceConfig, DeviceDescriptor, DeviceId,
    DeviceRegistry, Direction, InputBlock, LatencyInfo, MeasurementDevice, OutputSource, Scale,
    StreamConfig, StreamHandle, StreamUpdate,
};
use anecho_engine::generator::{GenLevel, GeneratorSpec, Signal};
use anecho_engine::{Engine, RtaConfig, StreamKind, StreamRequest};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// Wraps a backend and counts every device `start()`.
struct CountingBackend {
    inner: Qa40xBackend,
    starts: Arc<AtomicUsize>,
}

#[async_trait]
impl DeviceBackend for CountingBackend {
    fn kind(&self) -> BackendKind {
        self.inner.kind()
    }

    async fn enumerate(&self) -> Vec<DeviceDescriptor> {
        self.inner.enumerate().await
    }

    async fn open(&self, id: &DeviceId) -> anecho_device::Result<Box<dyn MeasurementDevice>> {
        let inner = self.inner.open(id).await?;
        Ok(Box::new(CountingDevice {
            inner,
            starts: self.starts.clone(),
        }))
    }
}

struct CountingDevice {
    inner: Box<dyn MeasurementDevice>,
    starts: Arc<AtomicUsize>,
}

impl std::fmt::Debug for CountingDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingDevice").finish_non_exhaustive()
    }
}

#[async_trait]
impl MeasurementDevice for CountingDevice {
    fn descriptor(&self) -> &DeviceDescriptor {
        self.inner.descriptor()
    }

    async fn configure(&self, cfg: DeviceConfig) -> anecho_device::Result<AppliedConfig> {
        self.inner.configure(cfg).await
    }

    async fn applied_config(&self) -> Option<AppliedConfig> {
        self.inner.applied_config().await
    }

    async fn start(
        &self,
        cfg: StreamConfig,
        input: mpsc::Sender<InputBlock>,
        output: Option<Box<dyn OutputSource>>,
    ) -> anecho_device::Result<StreamHandle> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.inner.start(cfg, input, output).await
    }

    async fn stop(&self, handle: StreamHandle) -> anecho_device::Result<()> {
        self.inner.stop(handle).await
    }

    async fn update_stream(
        &self,
        handle: StreamHandle,
        update: StreamUpdate,
    ) -> anecho_device::Result<()> {
        self.inner.update_stream(handle, update).await
    }

    async fn set_input_range(&self, index: usize) -> anecho_device::Result<()> {
        self.inner.set_input_range(index).await
    }

    fn scale(&self, direction: Direction) -> Scale {
        self.inner.scale(direction)
    }

    fn latency(&self) -> LatencyInfo {
        self.inner.latency()
    }
}

async fn engine_on_sim() -> (Arc<Engine>, u64, Arc<AtomicUsize>) {
    let starts = Arc::new(AtomicUsize::new(0));
    let backend = CountingBackend {
        inner: Qa40xBackend::empty().with_simulator(false),
        starts: starts.clone(),
    };
    let registry = DeviceRegistry::new().with_backend(Arc::new(backend));
    let engine = Engine::new(registry);
    let devices = engine.list_devices().await;
    let dev = &devices[0];
    let in_idx = dev
        .capabilities
        .input_ranges
        .iter()
        .position(|r| r.full_scale_dbv == 6.0)
        .unwrap();
    let out_idx = dev
        .capabilities
        .output_ranges
        .iter()
        .position(|r| r.full_scale_dbv == -2.0)
        .unwrap();
    let (session, _, _) = engine
        .open_session(
            &dev.id,
            DeviceConfig {
                input_range: Some(in_idx),
                output_range: Some(out_idx),
                ..DeviceConfig::with_sample_rate(48_000)
            },
        )
        .await
        .unwrap();
    (engine, session, starts)
}

fn sine(dbfs: f64) -> GeneratorSpec {
    GeneratorSpec {
        signal: Signal::Sine { hz: 1000.0 },
        level: GenLevel::PeakDbfs(dbfs),
        output_channels: vec![],
    }
}

/// Wait for `n` frames of `stream_id`, with a timeout.
async fn frames_of(
    rx: &mut tokio::sync::broadcast::Receiver<Arc<anecho_engine::Frame>>,
    stream_id: u32,
    n: usize,
) -> Vec<Arc<anecho_engine::Frame>> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while out.len() < n {
        let f = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("frames keep coming")
            .expect("channel open");
        if f.stream_id == stream_id {
            out.push(f);
        }
    }
    out
}

#[tokio::test]
async fn switching_streams_reuses_one_device_start() {
    let (engine, session, starts) = engine_on_sim().await;
    let mut frames = engine.frames();

    // LEVELS
    let mut req = StreamRequest::new(StreamKind::Levels);
    req.generator = Some(sine(-20.0));
    let levels = engine.start_stream(session, req).await.unwrap();
    let f = frames_of(&mut frames, levels.stream_id, 3).await;
    let rms = f[2].values[0];
    assert!((rms + 21.76).abs() < 1.0, "levels rms {rms} dBV");

    // RTA, FFT 4096
    let mut req = StreamRequest::new(StreamKind::Rta);
    req.generator = Some(sine(-20.0));
    req.rta = Some(RtaConfig {
        fft_length: 4096,
        ..Default::default()
    });
    let rta1 = engine.start_stream(session, req.clone()).await.unwrap();
    assert_ne!(rta1.stream_id, levels.stream_id);
    let f = frames_of(&mut frames, rta1.stream_id, 2).await;
    let peak = f[1].values[..rta1.values_per_channel as usize]
        .iter()
        .cloned()
        .fold(f32::MIN, f32::max);
    assert!((peak + 21.76).abs() < 2.0, "rta peak {peak} dBV");

    // RTA, FFT 8192
    req.rta = Some(RtaConfig {
        fft_length: 8192,
        ..Default::default()
    });
    let rta2 = engine.start_stream(session, req).await.unwrap();
    let f = frames_of(&mut frames, rta2.stream_id, 2).await;
    let peak = f[1].values[..rta2.values_per_channel as usize]
        .iter()
        .cloned()
        .fold(f32::MIN, f32::max);
    assert!((peak + 21.76).abs() < 2.0, "rta2 peak {peak} dBV");

    // SCOPE
    let mut req = StreamRequest::new(StreamKind::Scope);
    req.generator = Some(sine(-20.0));
    let scope = engine.start_stream(session, req).await.unwrap();
    let f = frames_of(&mut frames, scope.stream_id, 2).await;
    assert_eq!(f[0].channels, 2);

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "every switch must reuse the single device start"
    );
    engine.stop_stream(scope.stream_id).await.unwrap();
    engine.close_session(session).await.unwrap();
}

#[tokio::test]
async fn generator_hot_swap_changes_level_without_restart() {
    let (engine, session, starts) = engine_on_sim().await;
    let mut frames = engine.frames();

    let mut req = StreamRequest::new(StreamKind::Levels);
    req.generator = Some(sine(-20.0));
    let s1 = engine.start_stream(session, req).await.unwrap();
    let f = frames_of(&mut frames, s1.stream_id, 3).await;
    let rms = f[2].values[0];
    assert!((rms + 21.76).abs() < 1.0, "before swap: {rms} dBV");

    let mut req = StreamRequest::new(StreamKind::Levels);
    req.generator = Some(sine(-30.0));
    let s2 = engine.start_stream(session, req).await.unwrap();
    assert_ne!(s2.stream_id, s1.stream_id);
    let f = frames_of(&mut frames, s2.stream_id, 4).await;
    let rms = f[3].values[0];
    assert!((rms + 31.76).abs() < 1.0, "after swap: {rms} dBV");

    assert_eq!(starts.load(Ordering::SeqCst), 1);
    engine.close_session(session).await.unwrap();
}
