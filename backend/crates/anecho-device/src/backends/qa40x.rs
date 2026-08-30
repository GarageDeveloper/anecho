//! QuantAsylum QA402/QA403 through `qa40x-driver`.
//!
//! The QA40x is a half-duplex USB pipe driven block by block: each
//! `generate_and_capture` call sends a stimulus and returns the synchronous capture. The
//! adapter loops such calls to produce a stream; samples inside one call are contiguous and
//! sample-synchronous with the stimulus, but there is a short gap (USB turnaround + lead-in)
//! between two calls. `InputBlock::first_frame` counts captured frames only. Continuous,
//! gap-free streaming needs the driver's lower-level pump and is a phase 1 item.

use crate::backends::blocker::Blocker;
use crate::{
    AppliedConfig, BackendKind, Calibration, Capabilities, DeviceBackend, DeviceConfig,
    DeviceDescriptor, DeviceError, DeviceId, Direction, InputBlock, LatencyInfo, MeasurementDevice,
    OutputSource, Range, Result, Scale, StreamConfig, StreamHandle,
};
use async_trait::async_trait;
use qa40x_driver::{
    CalibrationSource, Channel, DeviceHandle, DeviceSource, InputGain, OutputGain, QA40xDevice,
    SampleRate, UsbDeviceSource,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

/// Frames generated+captured per driver call.
pub const CHUNK_FRAMES: usize = 8192;

pub struct Qa40xBackend {
    sources: Vec<Arc<dyn DeviceSource>>,
}

impl std::fmt::Debug for Qa40xBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self.sources.iter().map(|s| s.id().as_str()).collect();
        f.debug_struct("Qa40xBackend")
            .field("sources", &ids)
            .finish()
    }
}

impl Default for Qa40xBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Qa40xBackend {
    /// Physical units on the USB bus.
    pub fn new() -> Self {
        Self {
            sources: vec![Arc::new(UsbDeviceSource::new())],
        }
    }

    /// No USB at all (tests).
    pub fn empty() -> Self {
        Self { sources: vec![] }
    }

    pub fn with_source(mut self, source: Arc<dyn DeviceSource>) -> Self {
        self.sources.push(source);
        self
    }

    /// Add the embedded simulator as a unit with loopback wiring.
    #[cfg(feature = "qa40x-sim")]
    pub fn with_simulator(self, realtime: bool) -> Self {
        use qa40x_driver::discovery::{VirtualDeviceSource, VirtualUnit, demo_unit_options};
        let mut opts = demo_unit_options(0);
        opts.realtime = realtime;
        opts.loopback = true;
        self.with_source(Arc::new(VirtualDeviceSource::with_units(vec![
            VirtualUnit::new(opts),
        ])))
    }

    fn describe(d: &qa40x_driver::DeviceDescriptor) -> DeviceDescriptor {
        let c = &d.capabilities;
        let range = |dbv: &i32| Range {
            full_scale_dbv: *dbv as f32,
            label: format!("{dbv:+} dBV"),
        };
        let calibration = match &c.calibration {
            CalibrationSource::NominalFallback => Calibration::Factory {
                source: "nominal (calibration page unreadable)".into(),
            },
            CalibrationSource::User { label } => Calibration::Factory {
                source: label.clone(),
            },
            CalibrationSource::Unknown | CalibrationSource::FactoryEeprom { .. } => {
                Calibration::Factory {
                    source: "factory EEPROM".into(),
                }
            }
        };
        let transport = match &d.transport {
            qa40x_driver::Transport::Usb {
                bus_id, port_chain, ..
            } => format!("usb bus {bus_id} port {port_chain:?}"),
            qa40x_driver::Transport::Virtual => "simulator".into(),
        };
        DeviceDescriptor {
            id: DeviceId::new(BackendKind::Qa40x, d.id.as_str()),
            display_name: format!("{} {}", c.model_name, d.identity.serial),
            backend: BackendKind::Qa40x,
            transport,
            capabilities: Capabilities {
                sample_rates: c.sample_rates_hz.clone(),
                input_channels: c.input_channels as u16,
                output_channels: c.output_channels as u16,
                calibration,
                input_ranges: c.input_ranges_dbv.iter().map(range).collect(),
                output_ranges: c.output_ranges_dbv.iter().map(range).collect(),
                synchronous_io: true,
                nominal_latency_frames: None,
            },
        }
    }

    fn driver_id(id: &DeviceId) -> Option<qa40x_driver::DeviceId> {
        let rest = id.as_str().strip_prefix("qa40x/")?;
        Some(qa40x_driver::DeviceId::from_wire(rest))
    }
}

fn map_err(e: qa40x_driver::DeviceError) -> DeviceError {
    DeviceError::Backend(e.to_string())
}

fn map_qerr(e: qa40x_driver::QA40xError) -> DeviceError {
    DeviceError::Backend(e.to_string())
}

#[async_trait]
impl DeviceBackend for Qa40xBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Qa40x
    }

    async fn enumerate(&self) -> Vec<DeviceDescriptor> {
        let mut out = Vec::new();
        for s in &self.sources {
            match s.enumerate().await {
                Ok(list) => out.extend(list.iter().map(Self::describe)),
                Err(e) => log::warn!("qa40x source {}: {e}", s.id().as_str()),
            }
        }
        out
    }

    async fn open(&self, id: &DeviceId) -> Result<Box<dyn MeasurementDevice>> {
        let did = Self::driver_id(id).ok_or_else(|| DeviceError::NotFound(id.to_string()))?;
        let source = self
            .sources
            .iter()
            .find(|s| s.id().as_str() == did.source())
            .ok_or_else(|| DeviceError::NotFound(id.to_string()))?;
        let handle: DeviceHandle = Arc::new(Mutex::new(QA40xDevice::new()));
        let enriched = source.open(&did, &handle).await.map_err(map_err)?;
        Ok(Box::new(Qa40xDevice {
            descriptor: Self::describe(&enriched),
            handle,
            state: Mutex::new(State::default()),
            offsets_dbv: std::sync::Mutex::new((0.0, 0.0)),
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
    cancel: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

pub struct Qa40xDevice {
    descriptor: DeviceDescriptor,
    handle: DeviceHandle,
    state: Mutex<State>,
    /// (input, output) dBV offsets for the applied ranges; cached because `scale` is sync.
    offsets_dbv: std::sync::Mutex<(f32, f32)>,
}

impl std::fmt::Debug for Qa40xDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Qa40xDevice")
            .field("id", &self.descriptor.id)
            .finish_non_exhaustive()
    }
}

impl Qa40xDevice {
    fn range_dbv(ranges: &[Range], idx: Option<usize>, default: usize) -> Result<i32> {
        let i = idx.unwrap_or(default);
        ranges
            .get(i)
            .map(|r| r.full_scale_dbv as i32)
            .ok_or_else(|| DeviceError::UnsupportedConfig(format!("range index {i}")))
    }
}

#[async_trait]
impl MeasurementDevice for Qa40xDevice {
    fn descriptor(&self) -> &DeviceDescriptor {
        &self.descriptor
    }

    async fn configure(&self, cfg: DeviceConfig) -> Result<AppliedConfig> {
        let caps = &self.descriptor.capabilities;
        let mut st = self.state.lock().await;
        if st.running.is_some() {
            return Err(DeviceError::Busy);
        }
        let rate = SampleRate::from_hz(cfg.sample_rate)
            .filter(|_| caps.sample_rates.contains(&cfg.sample_rate))
            .ok_or_else(|| {
                DeviceError::UnsupportedConfig(format!("sample rate {} Hz", cfg.sample_rate))
            })?;
        // Safe defaults: widest input range, lowest output range.
        let in_idx = cfg
            .input_range
            .unwrap_or(caps.input_ranges.len().saturating_sub(1));
        let out_idx = cfg.output_range.unwrap_or(0);
        let in_dbv = Self::range_dbv(&caps.input_ranges, Some(in_idx), 0)?;
        let out_dbv = Self::range_dbv(&caps.output_ranges, Some(out_idx), 0)?;
        let in_gain = InputGain::from_dbv(in_dbv)
            .ok_or_else(|| DeviceError::UnsupportedConfig(format!("input range {in_dbv} dBV")))?;
        let out_gain = OutputGain::from_dbv(out_dbv)
            .ok_or_else(|| DeviceError::UnsupportedConfig(format!("output range {out_dbv} dBV")))?;
        for &c in cfg.input_channels.iter().chain(cfg.output_channels.iter()) {
            if c >= 2 {
                return Err(DeviceError::UnsupportedConfig(format!("channel {c}")));
            }
        }

        let dev = self.handle.lock().await;
        dev.set_sample_rate(rate).await.map_err(map_qerr)?;
        dev.set_input_gain(in_gain).await.map_err(map_qerr)?;
        dev.set_output_gain(out_gain).await.map_err(map_qerr)?;
        let (in_off, in_cal) = dev.input_dbv_offset(Channel::Left).await;
        let (out_off, out_cal) = dev.output_dbv_offset(Channel::Left).await;
        drop(dev);
        if !in_cal || !out_cal {
            log::warn!("{}: nominal calibration in use", self.descriptor.id);
        }

        let applied = AppliedConfig {
            sample_rate: cfg.sample_rate,
            input_range: Some(in_idx),
            output_range: Some(out_idx),
            input_channels: if cfg.input_channels.is_empty() {
                vec![0, 1]
            } else {
                cfg.input_channels
            },
            output_channels: if cfg.output_channels.is_empty() {
                vec![0, 1]
            } else {
                cfg.output_channels
            },
        };
        st.applied = Some(applied.clone());
        *self.offsets_dbv.lock().unwrap() = (in_off, out_off);
        Ok(applied)
    }

    async fn applied_config(&self) -> Option<AppliedConfig> {
        self.state.lock().await.applied.clone()
    }

    async fn start(
        &self,
        cfg: StreamConfig,
        input: mpsc::Sender<InputBlock>,
        output: Option<Box<dyn OutputSource>>,
    ) -> Result<StreamHandle> {
        let mut st = self.state.lock().await;
        if st.running.is_some() {
            return Err(DeviceError::Busy);
        }
        let applied = st.applied.clone().ok_or(DeviceError::NotConfigured)?;
        let handle = StreamHandle(st.next_handle);
        st.next_handle += 1;
        let cancel = Arc::new(AtomicBool::new(false));

        let worker = Worker {
            handle: self.handle.clone(),
            applied,
            blocker: Blocker::new(2, cfg.block_frames.max(1), input),
            cfg,
            source: output,
            cancel: cancel.clone(),
        };
        let task = tokio::spawn(worker.run());
        st.running = Some(Running {
            handle,
            cancel,
            task,
        });
        Ok(handle)
    }

    async fn stop(&self, handle: StreamHandle) -> Result<()> {
        let running = {
            let mut st = self.state.lock().await;
            match &st.running {
                Some(r) if r.handle == handle => st.running.take(),
                Some(_) => return Err(DeviceError::NoSuchStream),
                None => return Ok(()),
            }
        };
        if let Some(r) = running {
            r.cancel.store(true, Ordering::SeqCst);
            let _ = r.task.await;
        }
        Ok(())
    }

    fn scale(&self, direction: Direction) -> Scale {
        let (input, output) = *self.offsets_dbv.lock().unwrap();
        Scale::Volts {
            dbv_offset: match direction {
                Direction::Input => input,
                Direction::Output => output,
            },
        }
    }

    fn latency(&self) -> LatencyInfo {
        LatencyInfo {
            reported_frames: None,
            measured_frames: None,
        }
    }
}

struct Worker {
    handle: DeviceHandle,
    applied: AppliedConfig,
    cfg: StreamConfig,
    source: Option<Box<dyn OutputSource>>,
    blocker: Blocker,
    cancel: Arc<AtomicBool>,
}

impl Worker {
    async fn run(mut self) {
        let sr = self.applied.sample_rate;
        let mut source: Box<dyn OutputSource> = self
            .source
            .take()
            .unwrap_or_else(|| Box::new(crate::traits::Silence));
        let mut inter = vec![0f32; CHUNK_FRAMES * 2];
        let mut left = vec![0f32; CHUNK_FRAMES];
        let mut right = vec![0f32; CHUNK_FRAMES];
        let drive_l = self.applied.output_channels.contains(&0);
        let drive_r = self.applied.output_channels.contains(&1);
        let cap_l = self.applied.input_channels.contains(&0);
        let cap_r = self.applied.input_channels.contains(&1);

        while !self.cancel.load(Ordering::Relaxed) && !self.blocker.is_closed() {
            inter.iter_mut().for_each(|s| *s = 0.0);
            if self.cfg.generate {
                source.fill(&mut inter, 2, sr);
            }
            for (i, fr) in inter.chunks_exact(2).enumerate() {
                left[i] = if drive_l { fr[0] } else { 0.0 };
                right[i] = if drive_r { fr[1] } else { 0.0 };
            }
            let dev = self.handle.lock().await;
            let res = dev
                .generate_and_capture_cancellable(&left, &right, Some(&self.cancel))
                .await;
            drop(dev);
            let audio = match res {
                Ok(a) => a,
                Err(e) => {
                    if !self.cancel.load(Ordering::Relaxed) {
                        log::warn!("qa40x capture failed: {e}");
                    }
                    break;
                }
            };
            if !self.cfg.capture {
                continue;
            }
            let n = audio.left_channel.len().min(audio.right_channel.len());
            let mut out = Vec::with_capacity(n * 2);
            for i in 0..n {
                out.push(if cap_l { audio.left_channel[i] } else { 0.0 });
                out.push(if cap_r { audio.right_channel[i] } else { 0.0 });
            }
            self.blocker.push(&out);
        }
    }
}
