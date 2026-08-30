//! Generic sound cards through cpal.
//!
//! Hosts: WASAPI (shared mode — cpal 0.18 has no exclusive-mode support; see
//! `docs/decisions.md`), ASIO (feature `asio`), Core Audio, ALSA, JACK, PipeWire.
//!
//! cpal streams are not `Send` on every platform, so each running stream lives on its own
//! thread that builds, plays and finally drops the cpal streams.

use crate::backends::blocker::Blocker;
use crate::{
    AppliedConfig, BackendKind, Calibration, Capabilities, DeviceBackend, DeviceConfig,
    DeviceDescriptor, DeviceError, DeviceId, Direction, InputBlock, LatencyInfo, MeasurementDevice,
    OutputSource, Result, Scale, StreamConfig, StreamHandle,
};
use async_trait::async_trait;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tokio::sync::mpsc;

/// Sample rates Anecho cares about; a device advertises the subset it supports.
const CANDIDATE_RATES: &[u32] = &[44_100, 48_000, 88_200, 96_000, 176_400, 192_000, 384_000];

#[derive(Debug, Default, Clone)]
pub struct CpalBackend;

impl CpalBackend {
    pub fn new() -> Self {
        Self
    }

    fn hosts() -> Vec<cpal::Host> {
        cpal::available_hosts()
            .into_iter()
            .filter_map(|id| cpal::host_from_id(id).ok())
            .collect()
    }

    fn describe(host: &cpal::Host, dev: &cpal::Device) -> Option<DeviceDescriptor> {
        let name = dev.description().ok()?.name().to_string();
        let unit = dev.id().ok()?.id().to_string();
        let host_name = host.id().name().to_ascii_lowercase();
        let (in_rates, in_ch) = probe(dev.supported_input_configs().ok());
        let (out_rates, out_ch) = probe(dev.supported_output_configs().ok());
        if in_ch == 0 && out_ch == 0 {
            return None;
        }
        // Rates usable for capture (or for generation when the device has no input).
        let rates: Vec<u32> = CANDIDATE_RATES
            .iter()
            .copied()
            .filter(|r| {
                (in_ch == 0 || in_rates.contains(r)) && (out_ch == 0 || out_rates.contains(r))
            })
            .collect();
        Some(DeviceDescriptor {
            id: DeviceId::new(BackendKind::Cpal, &format!("{host_name}/{unit}")),
            display_name: name,
            backend: BackendKind::Cpal,
            transport: host.id().name().to_string(),
            capabilities: Capabilities {
                sample_rates: rates,
                input_channels: in_ch,
                output_channels: out_ch,
                calibration: Calibration::None,
                input_ranges: vec![],
                output_ranges: vec![],
                synchronous_io: false,
                nominal_latency_frames: None,
            },
        })
    }

    fn find(id: &DeviceId) -> Option<(cpal::Host, cpal::Device, DeviceDescriptor)> {
        for host in Self::hosts() {
            let Ok(devices) = host.devices() else {
                continue;
            };
            for dev in devices {
                if let Some(d) = Self::describe(&host, &dev)
                    && d.id == *id
                {
                    return Some((host, dev, d));
                }
            }
        }
        None
    }
}

fn probe<I>(configs: Option<I>) -> (Vec<u32>, u16)
where
    I: Iterator<Item = cpal::SupportedStreamConfigRange>,
{
    let mut rates = Vec::new();
    let mut channels = 0u16;
    if let Some(configs) = configs {
        for c in configs {
            channels = channels.max(c.channels());
            for &r in CANDIDATE_RATES {
                if r >= c.min_sample_rate() && r <= c.max_sample_rate() && !rates.contains(&r) {
                    rates.push(r);
                }
            }
        }
    }
    rates.sort_unstable();
    (rates, channels)
}

#[async_trait]
impl DeviceBackend for CpalBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Cpal
    }

    async fn enumerate(&self) -> Vec<DeviceDescriptor> {
        tokio::task::spawn_blocking(|| {
            let mut out = Vec::new();
            for host in Self::hosts() {
                let Ok(devices) = host.devices() else {
                    continue;
                };
                out.extend(devices.filter_map(|d| Self::describe(&host, &d)));
            }
            out
        })
        .await
        .unwrap_or_default()
    }

    async fn open(&self, id: &DeviceId) -> Result<Box<dyn MeasurementDevice>> {
        let id = id.clone();
        let found = tokio::task::spawn_blocking(move || Self::find(&id).map(|(_, _, d)| d))
            .await
            .map_err(|e| DeviceError::Backend(e.to_string()))?;
        let descriptor = found.ok_or_else(|| DeviceError::NotFound(id_display(self)))?;
        Ok(Box::new(CpalDevice {
            descriptor,
            state: Mutex::new(State::default()),
        }))
    }
}

fn id_display(_: &CpalBackend) -> String {
    "cpal device".into()
}

#[derive(Default)]
struct State {
    applied: Option<AppliedConfig>,
    running: Option<Running>,
    next_handle: u64,
}

struct Running {
    handle: StreamHandle,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

pub struct CpalDevice {
    descriptor: DeviceDescriptor,
    state: Mutex<State>,
}

impl std::fmt::Debug for CpalDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalDevice")
            .field("id", &self.descriptor.id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl MeasurementDevice for CpalDevice {
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    async fn configure(&self, cfg: DeviceConfig) -> Result<AppliedConfig> {
        let caps = &self.descriptor.capabilities;
        let mut st = self.state.lock().unwrap();
        if st.running.is_some() {
            return Err(DeviceError::Busy);
        }
        if !caps.sample_rates.contains(&cfg.sample_rate) {
            return Err(DeviceError::UnsupportedConfig(format!(
                "sample rate {} Hz",
                cfg.sample_rate
            )));
        }
        if cfg.input_range.is_some() || cfg.output_range.is_some() {
            return Err(DeviceError::UnsupportedConfig(
                "sound cards have no selectable ranges".into(),
            ));
        }
        let applied = AppliedConfig {
            sample_rate: cfg.sample_rate,
            input_range: None,
            output_range: None,
            input_channels: expand(cfg.input_channels, caps.input_channels)?,
            output_channels: expand(cfg.output_channels, caps.output_channels)?,
        };
        st.applied = Some(applied.clone());
        Ok(applied)
    }

    async fn applied_config(&self) -> Option<AppliedConfig> {
        self.state.lock().unwrap().applied.clone()
    }

    async fn start(
        &self,
        cfg: StreamConfig,
        input: mpsc::Sender<InputBlock>,
        output: Option<Box<dyn OutputSource>>,
    ) -> Result<StreamHandle> {
        let (applied, handle, stop) = {
            let mut st = self.state.lock().unwrap();
            if st.running.is_some() {
                return Err(DeviceError::Busy);
            }
            let applied = st.applied.clone().ok_or(DeviceError::NotConfigured)?;
            let handle = StreamHandle(st.next_handle);
            st.next_handle += 1;
            (applied, handle, Arc::new(AtomicBool::new(false)))
        };
        let caps = self.descriptor.capabilities.clone();
        let id = self.descriptor.id.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new()
            .name("anecho-cpal-stream".into())
            .spawn(move || {
                stream_thread(id, caps, applied, cfg, input, output, stop2, ready_tx);
            })
            .map_err(DeviceError::Io)?;

        let ready = tokio::task::spawn_blocking(move || ready_rx.recv())
            .await
            .map_err(|e| DeviceError::Backend(e.to_string()))?;
        match ready {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(DeviceError::Backend("stream thread died".into()));
            }
        }
        self.state.lock().unwrap().running = Some(Running {
            handle,
            stop,
            thread: Some(thread),
        });
        Ok(handle)
    }

    async fn stop(&self, handle: StreamHandle) -> Result<()> {
        let running = {
            let mut st = self.state.lock().unwrap();
            match &st.running {
                Some(r) if r.handle == handle => st.running.take(),
                Some(_) => return Err(DeviceError::NoSuchStream),
                None => return Ok(()),
            }
        };
        if let Some(mut r) = running {
            r.stop.store(true, Ordering::SeqCst);
            if let Some(t) = r.thread.take() {
                let _ = tokio::task::spawn_blocking(move || t.join()).await;
            }
        }
        Ok(())
    }

    fn scale(&self, _direction: Direction) -> Scale {
        Scale::Dbfs
    }

    fn latency(&self) -> LatencyInfo {
        LatencyInfo {
            reported_frames: None,
            measured_frames: None,
        }
    }
}

fn expand(requested: Vec<u16>, available: u16) -> Result<Vec<u16>> {
    if requested.is_empty() {
        return Ok((0..available).collect());
    }
    if let Some(bad) = requested.iter().find(|&&c| c >= available) {
        return Err(DeviceError::UnsupportedConfig(format!(
            "channel {bad} out of range (device has {available})"
        )));
    }
    Ok(requested)
}

#[allow(clippy::too_many_arguments)]
fn stream_thread(
    id: DeviceId,
    caps: Capabilities,
    applied: AppliedConfig,
    cfg: StreamConfig,
    input: mpsc::Sender<InputBlock>,
    output: Option<Box<dyn OutputSource>>,
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) {
    let Some((_host, dev, _)) = CpalBackend::find(&id) else {
        let _ = ready.send(Err(DeviceError::NotFound(id.to_string())));
        return;
    };
    let sample_rate = applied.sample_rate;
    let err_cb = |e: cpal::Error| log::warn!("cpal stream error: {e}");

    let mut streams: Vec<cpal::Stream> = Vec::new();

    if cfg.capture && caps.input_channels > 0 {
        let dev_ch = caps.input_channels;
        let sel = applied.input_channels.clone();
        let out_ch = sel.len() as u16;
        let mut blocker = Blocker::new(out_ch, cfg.block_frames, input);
        let mut scratch: Vec<f32> = Vec::new();
        let config = cpal::StreamConfig {
            channels: dev_ch,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let r = dev.build_input_stream(
            config,
            move |data: &[f32], _info| {
                if sel.len() == dev_ch as usize
                    && sel.iter().enumerate().all(|(i, &c)| i == c as usize)
                {
                    blocker.push(data);
                } else {
                    scratch.clear();
                    for frame in data.chunks_exact(dev_ch as usize) {
                        scratch.extend(sel.iter().map(|&c| frame[c as usize]));
                    }
                    blocker.push(&scratch);
                }
            },
            err_cb,
            None,
        );
        match r {
            Ok(s) => streams.push(s),
            Err(e) => {
                let _ = ready.send(Err(DeviceError::Backend(format!("input stream: {e}"))));
                return;
            }
        }
    }

    if cfg.generate && caps.output_channels > 0 {
        let dev_ch = caps.output_channels;
        let sel = applied.output_channels.clone();
        let sel_ch = sel.len() as u16;
        let mut source: Box<dyn OutputSource> =
            output.unwrap_or_else(|| Box::new(crate::traits::Silence));
        let mut scratch: Vec<f32> = Vec::new();
        let config = cpal::StreamConfig {
            channels: dev_ch,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let r = dev.build_output_stream(
            config,
            move |data: &mut [f32], _info| {
                let frames = data.len() / dev_ch as usize;
                scratch.clear();
                scratch.resize(frames * sel_ch as usize, 0.0);
                source.fill(&mut scratch, sel_ch, sample_rate);
                data.iter_mut().for_each(|s| *s = 0.0);
                for (f, frame) in data.chunks_exact_mut(dev_ch as usize).enumerate() {
                    for (k, &c) in sel.iter().enumerate() {
                        frame[c as usize] = scratch[f * sel_ch as usize + k];
                    }
                }
            },
            err_cb,
            None,
        );
        match r {
            Ok(s) => streams.push(s),
            Err(e) => {
                let _ = ready.send(Err(DeviceError::Backend(format!("output stream: {e}"))));
                return;
            }
        }
    }

    for s in &streams {
        if let Err(e) = s.play() {
            let _ = ready.send(Err(DeviceError::Backend(format!("play: {e}"))));
            return;
        }
    }
    let _ = ready.send(Ok(()));

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(streams);
}
