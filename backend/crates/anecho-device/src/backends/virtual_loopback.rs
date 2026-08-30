//! In-process virtual device: whatever is generated comes back on the inputs after a fixed
//! delay and gain, plus optional white noise. Used by engine/server tests so that the whole
//! stack can be exercised without hardware, cpal or the QA40x simulator.

use crate::backends::blocker::Blocker;
use crate::{
    AppliedConfig, BackendKind, Calibration, Capabilities, DeviceBackend, DeviceConfig,
    DeviceDescriptor, DeviceError, DeviceId, Direction, InputBlock, LatencyInfo, MeasurementDevice,
    OutputSource, Result, Scale, StreamConfig, StreamHandle,
};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Behaviour of the virtual loopback.
#[derive(Debug, Clone)]
pub struct LoopbackOptions {
    pub channels: u16,
    pub sample_rates: Vec<u32>,
    /// Output→input delay, in frames.
    pub latency_frames: u32,
    pub gain_db: f32,
    /// Peak amplitude of additive white noise (0 = none).
    pub noise_peak: f32,
    /// Pace the stream at the sample rate (true) or run as fast as the consumer drains it.
    pub realtime: bool,
}

impl Default for LoopbackOptions {
    fn default() -> Self {
        Self {
            channels: 2,
            sample_rates: vec![44_100, 48_000, 96_000, 192_000],
            latency_frames: 1200,
            gain_db: 0.0,
            noise_peak: 0.0,
            realtime: false,
        }
    }
}

pub const UNIT_NAME: &str = "loopback";

#[derive(Debug, Clone, Default)]
pub struct VirtualLoopbackBackend {
    options: LoopbackOptions,
}

impl VirtualLoopbackBackend {
    pub fn new(options: LoopbackOptions) -> Self {
        Self { options }
    }

    fn descriptor(&self) -> DeviceDescriptor {
        DeviceDescriptor {
            id: DeviceId::new(BackendKind::Virtual, UNIT_NAME),
            display_name: "Virtual loopback".into(),
            backend: BackendKind::Virtual,
            transport: "in-process".into(),
            capabilities: Capabilities {
                sample_rates: self.options.sample_rates.clone(),
                input_channels: self.options.channels,
                output_channels: self.options.channels,
                calibration: Calibration::None,
                input_ranges: vec![],
                output_ranges: vec![],
                synchronous_io: true,
                nominal_latency_frames: Some(self.options.latency_frames),
            },
        }
    }
}

#[async_trait]
impl DeviceBackend for VirtualLoopbackBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Virtual
    }

    async fn enumerate(&self) -> Vec<DeviceDescriptor> {
        vec![self.descriptor()]
    }

    async fn open(&self, id: &DeviceId) -> Result<Box<dyn MeasurementDevice>> {
        let d = self.descriptor();
        if *id != d.id {
            return Err(DeviceError::NotFound(id.to_string()));
        }
        Ok(Box::new(VirtualLoopbackDevice {
            descriptor: d,
            options: self.options.clone(),
            state: Mutex::new(State::default()),
        }))
    }
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

pub struct VirtualLoopbackDevice {
    descriptor: DeviceDescriptor,
    options: LoopbackOptions,
    state: Mutex<State>,
}

impl std::fmt::Debug for VirtualLoopbackDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualLoopbackDevice")
            .field("id", &self.descriptor.id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl MeasurementDevice for VirtualLoopbackDevice {
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    async fn configure(&self, cfg: DeviceConfig) -> Result<AppliedConfig> {
        let mut st = self.state.lock().unwrap();
        if st.running.is_some() {
            return Err(DeviceError::Busy);
        }
        if !self.options.sample_rates.contains(&cfg.sample_rate) {
            return Err(DeviceError::UnsupportedConfig(format!(
                "sample rate {} Hz",
                cfg.sample_rate
            )));
        }
        let all: Vec<u16> = (0..self.options.channels).collect();
        let applied = AppliedConfig {
            sample_rate: cfg.sample_rate,
            input_range: None,
            output_range: None,
            input_channels: if cfg.input_channels.is_empty() {
                all.clone()
            } else {
                cfg.input_channels
            },
            output_channels: if cfg.output_channels.is_empty() {
                all
            } else {
                cfg.output_channels
            },
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
        let mut st = self.state.lock().unwrap();
        if st.running.is_some() {
            return Err(DeviceError::Busy);
        }
        let applied = st.applied.clone().ok_or(DeviceError::NotConfigured)?;
        let stop = Arc::new(AtomicBool::new(false));
        let handle = StreamHandle(st.next_handle);
        st.next_handle += 1;

        let blocker = Blocker::new(self.options.channels, cfg_block_frames(&cfg), input);
        let blocker = if self.options.realtime {
            blocker
        } else {
            blocker.blocking()
        };
        let worker = Worker {
            options: self.options.clone(),
            sample_rate: applied.sample_rate,
            channels: self.options.channels,
            cfg,
            source: output,
            blocker,
            stop: stop.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("anecho-virtual-loopback".into())
            .spawn(move || worker.run())
            .map_err(DeviceError::Io)?;
        st.running = Some(Running {
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
            reported_frames: Some(self.options.latency_frames),
            measured_frames: None,
        }
    }
}

struct Worker {
    options: LoopbackOptions,
    sample_rate: u32,
    channels: u16,
    cfg: StreamConfig,
    source: Option<Box<dyn OutputSource>>,
    blocker: Blocker,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn run(mut self) {
        let ch = self.channels as usize;
        let block_frames = self.cfg.block_frames.max(1) as usize;
        let mut blocker = self.blocker;

        let gain = 10f32.powf(self.options.gain_db / 20.0);
        let delay = self.options.latency_frames as usize * ch;
        let mut line = vec![0f32; delay + block_frames * ch];
        let mut out = vec![0f32; block_frames * ch];
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        let mut source = self
            .source
            .take()
            .unwrap_or_else(|| Box::new(crate::traits::Silence));

        let block_dur = Duration::from_secs_f64(block_frames as f64 / self.sample_rate as f64);
        let start = Instant::now();
        let mut n: u64 = 0;

        while !self.stop.load(Ordering::Relaxed) && !blocker.is_closed() {
            out.iter_mut().for_each(|s| *s = 0.0);
            if self.cfg.generate {
                source.fill(&mut out, self.channels, self.sample_rate);
            }
            // Delay line: append the new output, read what was written `delay` samples ago.
            line.copy_within(block_frames * ch.., 0);
            line[delay..].copy_from_slice(&out);
            let mut captured: Vec<f32> = line[..block_frames * ch].to_vec();
            for s in &mut captured {
                *s *= gain;
                if self.options.noise_peak > 0.0 {
                    rng ^= rng << 13;
                    rng ^= rng >> 7;
                    rng ^= rng << 17;
                    let u = (rng >> 11) as f32 / (1u64 << 53) as f32; // [0,1)
                    *s += (u * 2.0 - 1.0) * self.options.noise_peak;
                }
            }
            if self.cfg.capture {
                blocker.push(&captured);
            }
            n += 1;
            if self.options.realtime {
                let due = start + block_dur * n as u32;
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        }
    }
}

fn cfg_block_frames(cfg: &StreamConfig) -> u32 {
    cfg.block_frames.max(1)
}
