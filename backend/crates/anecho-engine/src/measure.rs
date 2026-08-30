//! One-shot measurements: drive the outputs, capture, analyse, return a typed result.

use crate::generator::{GeneratorSpec, Signal};
use crate::{Engine, EngineError, Result};
use anecho_device::{DeviceError, Direction, InputBlock, Scale, StreamConfig};
use anecho_dsp::{Averager, Averaging, Imd, RealSpectrum, Thd, ThdOptions, Window};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureKind {
    Thd,
    ImdSmpte,
    ImdCcif,
}

#[derive(Debug, Clone)]
pub struct MeasureRequest {
    pub kind: MeasureKind,
    pub generator: Option<GeneratorSpec>,
    /// 0 = default (65536).
    pub fft_length: usize,
    pub window: Window,
    /// Captures averaged; 0 = default (4).
    pub averages: u32,
    /// THD: highest harmonic; 0 = default (9).
    pub max_harmonic: u32,
    /// THD+N band; `None` = 20 Hz to min(20 kHz, 0.45 fs).
    pub band_hz: Option<(f64, f64)>,
}

/// Result for one captured channel. `fundamental_level` is in the session's input scale
/// (dBFS_rms or dBV); everything else is relative or a percentage.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDistortion {
    pub fundamental_hz: f64,
    pub fundamental_level: f64,
    pub thd_pct: f64,
    pub thd_db: f64,
    pub thd_n_pct: f64,
    pub thd_n_db: f64,
    pub harmonics: Vec<anecho_dsp::Harmonic>,
    pub noise_floor_db: f64,
    pub imd_pct: f64,
    pub imd_db: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeasureResult {
    pub kind: MeasureKind,
    pub sample_rate: u32,
    pub scale: Scale,
    pub per_channel: Vec<ChannelDistortion>,
}

impl Engine {
    /// Run a one-shot measurement on a session. Refused while a stream is running.
    pub async fn measure(&self, session_id: u64, req: MeasureRequest) -> Result<MeasureResult> {
        let fft_length = if req.fft_length == 0 {
            65_536
        } else {
            req.fft_length
        };
        if !fft_length.is_power_of_two() || fft_length < 256 {
            return Err(EngineError::BadRequest(
                "fft_length must be a power of two >= 256".into(),
            ));
        }
        let averages = if req.averages == 0 { 4 } else { req.averages } as usize;

        // Claim the session.
        let (device, applied) = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get_mut(&session_id)
                .ok_or(EngineError::NoSuchSession(session_id))?;
            if session.stream.is_some() || session.measuring {
                return Err(EngineError::StreamRunning(session_id));
            }
            session.measuring = true;
            let applied = session
                .device
                .applied_config()
                .await
                .ok_or(DeviceError::NotConfigured)?;
            (session.device.clone(), applied)
        };
        let result = self
            .measure_inner(session_id, &device, &applied, req, fft_length, averages)
            .await;
        if let Some(s) = self.sessions.lock().await.get_mut(&session_id) {
            s.measuring = false;
        }
        result
    }

    async fn measure_inner(
        &self,
        session_id: u64,
        device: &std::sync::Arc<dyn anecho_device::MeasurementDevice>,
        applied: &anecho_device::AppliedConfig,
        req: MeasureRequest,
        fft_length: usize,
        averages: usize,
    ) -> Result<MeasureResult> {
        let channels = applied.input_channels.len();
        if channels == 0 {
            return Err(EngineError::BadRequest(
                "device has no input channel".into(),
            ));
        }
        let sample_rate = applied.sample_rate;
        // Snap generator tones to the FFT bin grid (k · fs / N): a tone centred on a bin
        // has no scalloping loss and no leakage skirt, which is what makes THD+N reach the
        // converter's noise floor instead of the window's main lobe (REW does the same
        // for its distortion figures). The reported fundamental is the snapped one.
        let bin_hz = sample_rate as f64 / fft_length as f64;
        let snap = |hz: f64| ((hz / bin_hz).round().max(1.0)) * bin_hz;
        let mut req = req;
        if let Some(g) = req.generator.as_mut() {
            match &mut g.signal {
                Signal::Sine { hz } | Signal::Square { hz } => *hz = snap(*hz),
                Signal::DualTone { f1, f2, .. } => {
                    *f1 = snap(*f1);
                    *f2 = snap(*f2);
                }
                _ => {}
            }
        }
        let hint = req.generator.as_ref().and_then(|g| match g.signal {
            Signal::Sine { hz } | Signal::Square { hz } => Some(hz),
            _ => None,
        });
        let (f1, f2) = match req.generator.as_ref().map(|g| &g.signal) {
            Some(Signal::DualTone { f1, f2, .. }) => (*f1, *f2),
            _ => match req.kind {
                MeasureKind::ImdCcif => (19_000.0, 20_000.0),
                _ => (60.0, 7000.0),
            },
        };
        let output = match req.generator.clone() {
            Some(spec) => Some(
                Self::resolve_generator(session_id, device, applied, spec, &self.events).await?,
            ),
            None => None,
        };
        let (tx, mut rx) = mpsc::channel::<InputBlock>(8);
        let handle = device
            .start(
                StreamConfig {
                    block_frames: fft_length as u32,
                    capture: true,
                    generate: output.is_some(),
                },
                tx,
                output,
            )
            .await?;

        // Discard the first block (range relays, generator ramp-in, loopback latency), then
        // average `averages` full-length blocks.
        let mut fft = RealSpectrum::new(fft_length);
        let mut averagers: Vec<Averager> = (0..channels)
            .map(|_| Averager::new(Averaging::Linear { n: averages as u32 }))
            .collect();
        let mut last_block: Option<InputBlock> = None;
        let mut collected = 0usize;
        let mut skipped = false;
        let outcome: Result<()> = loop {
            let Some(block) = rx.recv().await else {
                break Err(DeviceError::Disconnected.into());
            };
            if !skipped {
                skipped = true;
                continue;
            }
            let ch = block.channels as usize;
            let mut mono = vec![0f32; fft_length];
            for c in 0..channels.min(ch) {
                for (i, frame) in block.samples.chunks_exact(ch).enumerate() {
                    mono[i] = frame[c];
                }
                let power = fft.power(&mono, req.window);
                averagers[c].push(&power);
            }
            last_block = Some(block);
            collected += 1;
            if collected >= averages {
                break Ok(());
            }
        };
        // Close our side first: a backend that blocks on a full channel (the non-realtime
        // virtual loopback) must see it closed before `stop` joins its worker.
        drop(rx);
        let _ = device.stop(handle).await;
        outcome?;

        let scale = device.scale(Direction::Input);
        let offset = match scale {
            Scale::Dbfs => 0.0,
            Scale::Volts { dbv_offset } => dbv_offset as f64,
        };
        let opts = ThdOptions {
            window: req.window,
            max_harmonic: if req.max_harmonic == 0 {
                9
            } else {
                req.max_harmonic
            },
            fundamental_hint: hint,
            band_hz: req.band_hz,
        };
        let last = last_block.expect("at least one block collected");
        let ch = last.channels as usize;
        let mut per_channel = Vec::with_capacity(channels);
        for (c, averager) in averagers.iter().enumerate().take(channels) {
            let power = averager.current();
            let thd = Thd::analyze_power(power, sample_rate as f64, &opts);
            let (imd_pct, imd_db) = match req.kind {
                MeasureKind::Thd => (0.0, f64::NEG_INFINITY),
                MeasureKind::ImdSmpte | MeasureKind::ImdCcif => {
                    let mono: Vec<f32> = last.samples.iter().skip(c).step_by(ch).copied().collect();
                    let r = if req.kind == MeasureKind::ImdSmpte {
                        Imd::smpte(&mono, sample_rate as f64, req.window, f1, f2)
                    } else {
                        Imd::ccif(&mono, sample_rate as f64, req.window, f1, f2)
                    };
                    (r.imd_pct, r.imd_db)
                }
            };
            per_channel.push(ChannelDistortion {
                fundamental_hz: thd.fundamental_hz,
                // anecho-dsp reports the fundamental as a peak level; the session scale is RMS.
                fundamental_level: anecho_dsp::units::sine_peak_to_rms_db(thd.fundamental_level_db)
                    + offset,
                thd_pct: thd.thd_pct,
                thd_db: thd.thd_db,
                thd_n_pct: thd.thd_n_pct,
                thd_n_db: thd.thd_n_db,
                harmonics: thd.harmonics,
                noise_floor_db: thd.noise_floor_db - thd.fundamental_level_db,
                imd_pct,
                imd_db,
            });
        }
        Ok(MeasureResult {
            kind: req.kind,
            sample_rate,
            scale,
            per_channel,
        })
    }
}
