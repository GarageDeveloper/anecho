//! QuantAsylum QA402/QA403 through `qa40x-driver`.
//!
//! The QA40x is a half-duplex USB pipe driven block by block: each
//! `generate_and_capture` call sends a stimulus and returns the synchronous capture. The
//! adapter loops such calls to produce a stream; samples inside one call are contiguous and
//! sample-synchronous with the stimulus, but there is a short gap (USB turnaround + lead-in)
//! between two calls. `InputBlock::first_frame` counts captured frames only. Continuous,
//! gap-free streaming needs the driver's lower-level pump and is a phase 1 item.
//!
//! Round-trip latency: the driver returns the capture window aligned on the *start of the
//! stimulus*, so the first `L` samples of every call are the previous silence and the last
//! `L` samples of the stimulus never come back. When generating, the adapter measures `L`
//! once at stream start (a short burst, first sample above threshold), then pads every
//! stimulus with `L` trailing zeros and drops the first `L` captured samples, so each block
//! is pure, aligned stimulus response.

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

/// Default frames generated+captured per driver call (when the stream asks for smaller
/// blocks). Larger requested blocks raise the chunk so that **one block is always one
/// contiguous capture**: analyses spanning a whole FFT must never straddle the gap
/// between two half-duplex calls.
pub const CHUNK_FRAMES: usize = 8192;
/// Upper bound of a single call (USB queue depth / memory); 2^18 frames ≈ 5.5 s at 48 kHz.
pub const MAX_CHUNK_FRAMES: usize = 1 << 18;
/// Chirp used to measure the round-trip latency at stream start (peak, full scale = 1).
const LATENCY_PROBE_PEAK: f32 = 0.1;
/// Capture window of the probe.
const LATENCY_PROBE_FRAMES: usize = 4096;
/// Length of the chirp itself; the rest of the window is silence for the echo to land in.
const LATENCY_CHIRP_FRAMES: usize = 1024;
/// Chirp band (Hz). Broadband so that the cross-correlation peak is unambiguous.
const LATENCY_CHIRP_BAND: (f32, f32) = (200.0, 12_000.0);
/// Below this expected captured peak (dBFS) the probe cannot be trusted.
const LATENCY_MIN_EXPECTED_DBFS: f32 = -110.0;
/// Upper bound on a plausible latency; beyond that the probe is considered failed.
const LATENCY_MAX_FRAMES: usize = LATENCY_PROBE_FRAMES - LATENCY_CHIRP_FRAMES;
/// Correlation peak must exceed the mean absolute correlation by this factor.
const LATENCY_PEAK_TO_MEAN: f32 = 8.0;

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
            firmware_version: d.identity.firmware_version.map(|v| v.to_string()),
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
        // The interface is claimed exclusively; a handle that was just dropped (previous
        // session) may still be releasing it, so retry once after a short pause.
        let enriched = match source.open(&did, &handle).await {
            Ok(d) => d,
            Err(e) if e.to_string().to_ascii_lowercase().contains("claim") => {
                log::warn!("{id}: {e}; retrying in 500 ms");
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                source.open(&did, &handle).await.map_err(map_err)?
            }
            Err(e) => return Err(map_err(e)),
        };
        Ok(Box::new(Qa40xDevice {
            descriptor: Self::describe(&enriched),
            handle,
            state: Mutex::new(State::default()),
            offsets_dbv: Arc::new(std::sync::Mutex::new((0.0, 0.0))),
            measured_latency: Arc::new(std::sync::Mutex::new(None)),
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
    /// (input, output) dBV offsets for the applied ranges; cached because `scale` is sync,
    /// shared with the stream worker which stamps every block with the input offset.
    offsets_dbv: Arc<std::sync::Mutex<(f32, f32)>>,
    /// Round-trip latency measured at the last generating stream start, in frames.
    measured_latency: Arc<std::sync::Mutex<Option<usize>>>,
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
            measured_latency: self.measured_latency.clone(),
            offsets_dbv: self.offsets_dbv.clone(),
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
            measured_frames: self.measured_latency.lock().unwrap().map(|l| l as f64),
        }
    }

    /// Range write between two capture chunks: the worker only holds the device mutex
    /// during `generate_and_capture`, so this waits for the current chunk, writes the range
    /// (the driver handles the attenuator-out quirk) and refreshes the cached dBV offset.
    async fn set_input_range(&self, index: usize) -> Result<()> {
        let caps = &self.descriptor.capabilities;
        let dbv = Self::range_dbv(&caps.input_ranges, Some(index), 0)?;
        let gain = InputGain::from_dbv(dbv)
            .ok_or_else(|| DeviceError::UnsupportedConfig(format!("input range {dbv} dBV")))?;
        let dev = self.handle.lock().await;
        dev.set_input_gain(gain).await.map_err(map_qerr)?;
        let (in_off, _) = dev.input_dbv_offset(Channel::Left).await;
        drop(dev);
        self.offsets_dbv.lock().unwrap().0 = in_off;
        let mut st = self.state.lock().await;
        if let Some(a) = st.applied.as_mut() {
            a.input_range = Some(index);
        }
        Ok(())
    }
}

struct Worker {
    handle: DeviceHandle,
    applied: AppliedConfig,
    cfg: StreamConfig,
    source: Option<Box<dyn OutputSource>>,
    blocker: Blocker,
    cancel: Arc<AtomicBool>,
    measured_latency: Arc<std::sync::Mutex<Option<usize>>>,
    offsets_dbv: Arc<std::sync::Mutex<(f32, f32)>>,
}

/// Linear chirp, `LATENCY_CHIRP_FRAMES` long, zero-padded to `LATENCY_PROBE_FRAMES`.
fn latency_chirp(sample_rate: u32) -> Vec<f32> {
    let sr = sample_rate as f32;
    let (f0, f1) = LATENCY_CHIRP_BAND;
    let f1 = f1.min(sr * 0.4);
    let dur = LATENCY_CHIRP_FRAMES as f32 / sr;
    let k = (f1 - f0) / dur;
    let mut v = vec![0f32; LATENCY_PROBE_FRAMES];
    for (i, s) in v.iter_mut().enumerate().take(LATENCY_CHIRP_FRAMES) {
        let t = i as f32 / sr;
        let phase = std::f32::consts::TAU * (f0 * t + 0.5 * k * t * t);
        // Hann-shaped edges avoid clicks and sharpen the correlation peak.
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / LATENCY_CHIRP_FRAMES as f32).cos();
        *s = LATENCY_PROBE_PEAK * w * phase.sin();
    }
    v
}

/// Lag maximising the cross-correlation of `captured` with `reference`, if the peak is
/// clearly above the background.
fn best_lag(reference: &[f32], captured: &[f32], max_lag: usize) -> Option<usize> {
    let n = LATENCY_CHIRP_FRAMES.min(reference.len());
    let mut corr = Vec::with_capacity(max_lag + 1);
    for lag in 0..=max_lag {
        if lag + n > captured.len() {
            break;
        }
        let c: f32 = reference[..n]
            .iter()
            .zip(&captured[lag..lag + n])
            .map(|(a, b)| a * b)
            .sum();
        corr.push(c.abs());
    }
    let (best, peak) = corr
        .iter()
        .copied()
        .enumerate()
        .fold((0usize, 0f32), |m, (i, c)| if c > m.1 { (i, c) } else { m });
    let mean = corr.iter().sum::<f32>() / corr.len() as f32;
    (peak > 0.0 && peak > LATENCY_PEAK_TO_MEAN * mean).then_some(best)
}

/// Send a short chirp on the driven channels and measure when it comes back.
async fn probe_latency(
    handle: &DeviceHandle,
    cancel: &AtomicBool,
    sample_rate: u32,
    drive_l: bool,
    drive_r: bool,
    (in_off, out_off): (f32, f32),
) -> Option<usize> {
    // The chirp is generated on the output range and captured on the input range: its
    // expected captured peak is the probe peak shifted by the offset difference.
    let expected_dbfs = 20.0 * LATENCY_PROBE_PEAK.log10() + out_off - in_off;
    if expected_dbfs < LATENCY_MIN_EXPECTED_DBFS {
        log::warn!(
            "latency probe skipped: expected loopback level {expected_dbfs:.1} dBFS is too low"
        );
        return None;
    }
    let chirp = latency_chirp(sample_rate);
    let zero = vec![0f32; LATENCY_PROBE_FRAMES];
    let dev = handle.lock().await;
    let audio = dev
        .generate_and_capture_cancellable(
            if drive_l { &chirp } else { &zero },
            if drive_r { &chirp } else { &zero },
            Some(cancel),
        )
        .await
        .ok()?;
    drop(dev);
    let l = drive_l
        .then(|| best_lag(&chirp, &audio.left_channel, LATENCY_MAX_FRAMES))
        .flatten();
    let r = drive_r
        .then(|| best_lag(&chirp, &audio.right_channel, LATENCY_MAX_FRAMES))
        .flatten();
    match (l, r) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

impl Worker {
    async fn run(mut self) {
        let sr = self.applied.sample_rate;
        let mut source: Box<dyn OutputSource> = self
            .source
            .take()
            .unwrap_or_else(|| Box::new(crate::traits::Silence));
        let drive_l = self.applied.output_channels.contains(&0);
        let drive_r = self.applied.output_channels.contains(&1);
        let cap_l = self.applied.input_channels.contains(&0);
        let cap_r = self.applied.input_channels.contains(&1);

        // Latency compensation only matters when we drive the outputs.
        let offsets = *self.offsets_dbv.lock().unwrap();
        let pad = if self.cfg.generate && (drive_l || drive_r) {
            match probe_latency(
                &self.handle,
                &self.cancel,
                self.applied.sample_rate,
                drive_l,
                drive_r,
                offsets,
            )
            .await
            {
                Some(l) => {
                    log::info!("qa40x round-trip latency: {l} frames");
                    *self.measured_latency.lock().unwrap() = Some(l);
                    l
                }
                None => {
                    log::warn!(
                        "qa40x latency probe found no loopback signal; blocks are not latency-aligned"
                    );
                    0
                }
            }
        } else {
            0
        };
        if self.cancel.load(Ordering::Relaxed) {
            return;
        }

        // One chunk per requested block (rounded up to the default), so a block never
        // contains a chunk boundary.
        let block = self.cfg.block_frames.max(1) as usize;
        let chunk = if block <= CHUNK_FRAMES {
            CHUNK_FRAMES
        } else {
            block.min(MAX_CHUNK_FRAMES)
        };
        let mut inter = vec![0f32; chunk * 2];
        let mut left = vec![0f32; chunk + pad];
        let mut right = vec![0f32; chunk + pad];

        while !self.cancel.load(Ordering::Relaxed) && !self.blocker.is_closed() {
            inter.iter_mut().for_each(|s| *s = 0.0);
            if self.cfg.generate {
                source.fill(&mut inter, 2, sr);
            }
            for (i, fr) in inter.as_chunks::<2>().0.iter().enumerate() {
                left[i] = if drive_l { fr[0] } else { 0.0 };
                right[i] = if drive_r { fr[1] } else { 0.0 };
            }
            // Trailing zeros: the stimulus tail must have time to come back.
            left[chunk..].iter_mut().for_each(|s| *s = 0.0);
            right[chunk..].iter_mut().for_each(|s| *s = 0.0);
            // The device mutex stays held until the chunk's blocks are handed over: a range
            // write waiting on the mutex (`set_input_range`) then always lands *between* the
            // blocks of two chunks, so every block is captured entirely on one range.
            let dev = self.handle.lock().await;
            let res = dev
                .generate_and_capture_cancellable(&left, &right, Some(&self.cancel))
                .await;
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
            let start = pad.min(n);
            let end = (start + chunk).min(n);
            let mut out = Vec::with_capacity((end - start) * 2);
            for i in start..end {
                out.push(if cap_l { audio.left_channel[i] } else { 0.0 });
                out.push(if cap_r { audio.right_channel[i] } else { 0.0 });
            }
            // Stamp the blocks with the input offset the chunk was captured under.
            let in_off = self.offsets_dbv.lock().unwrap().0;
            self.blocker.set_scale(Scale::Volts { dbv_offset: in_off });
            self.blocker.push(&out);
            drop(dev);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chirp_lag_is_recovered_in_noise() {
        let chirp = latency_chirp(48_000);
        for lag in [0usize, 48, 1200, 2500] {
            let mut captured = vec![0f32; LATENCY_PROBE_FRAMES];
            let mut rng = 0x1234_5678u32;
            for (i, s) in captured.iter_mut().enumerate() {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                let noise = (rng as f32 / u32::MAX as f32 - 0.5) * 0.02; // -40 dBFS peak
                let echo = if i >= lag && i - lag < LATENCY_CHIRP_FRAMES {
                    0.01 * chirp[i - lag] / LATENCY_PROBE_PEAK
                } else {
                    0.0
                };
                *s = noise + echo; // echo at -40 dBFS peak, level with the noise
            }
            assert_eq!(
                best_lag(&chirp, &captured, LATENCY_MAX_FRAMES),
                Some(lag),
                "lag {lag}"
            );
        }
        // Pure noise: no lag.
        let noise: Vec<f32> = (0..LATENCY_PROBE_FRAMES)
            .map(|i| ((i * 7919) % 1000) as f32 / 1000.0 - 0.5)
            .collect();
        assert_eq!(best_lag(&chirp, &noise, LATENCY_MAX_FRAMES), None);
    }
}
