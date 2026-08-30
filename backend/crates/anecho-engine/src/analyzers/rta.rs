//! Real-time analyzer: FFT per channel, averaging, then a display axis.
//!
//! **Value convention** — the same as the LEVELS stream: every axis point carries an RMS
//! level in dB, `dBFS_rms` on uncalibrated devices or dBV when the device is calibrated (the
//! input `dbv_offset` is applied here, never by a client). A sine at −20 dBV therefore
//! peaks at −20 dBV on the RTA, and a −20 dBFS-peak sine reads −23.01 dBFS_rms.
//!
//! - Log axis: for each display point, the **maximum** per-bin power over the bins in the
//!   point's cell (`anecho_dsp::LogAxis::decimate`), so narrow lines survive decimation.
//! - Octave bands: band power `Σ P_k / ENBW` (`anecho_dsp::OctaveBands::band_powers`),
//!   i.e. the RMS² of everything inside the band — a sine reads its RMS level, noise reads
//!   its band RMS.
//!
//! FFT frames are assembled from consecutive input blocks, hop = FFT length (no overlap).
//! On the QA40x, blocks come from half-duplex chunks with a short gap between chunks; a
//! frame longer than a chunk necessarily straddles a gap. Magnitudes of stationary signals
//! are unaffected; phase is not meaningful across a gap.

use super::Reading;
use anecho_device::InputBlock;
use anecho_dsp::{Averager, Averaging, LogAxis, OctaveBands, RealSpectrum, Window};

/// Display axis of the RTA.
#[derive(Debug, Clone, PartialEq)]
pub enum RtaAxis {
    /// `points` logarithmically spaced frequencies between `min_hz` and `max_hz`.
    Log {
        min_hz: f64,
        max_hz: f64,
        points: usize,
    },
    /// Fractional-octave bands (1 = 1/1, 3 = 1/3, ...), base 2, centred on 1 kHz.
    Octave {
        fraction: u32,
        min_hz: f64,
        max_hz: f64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RtaConfig {
    pub fft_length: usize,
    pub window: Window,
    pub averaging: Averaging,
    pub axis: RtaAxis,
    /// Maximum emission rate; 0 = one frame per FFT.
    pub update_rate_hz: f32,
}

impl Default for RtaConfig {
    fn default() -> Self {
        Self {
            fft_length: 16_384,
            window: Window::Hann,
            averaging: Averaging::None,
            axis: RtaAxis::Log {
                min_hz: 20.0,
                max_hz: 20_000.0,
                points: 1000,
            },
            update_rate_hz: 0.0,
        }
    }
}

enum Axis {
    Log(LogAxis),
    Octave(OctaveBands),
}

pub struct Rta {
    fft: RealSpectrum,
    window: Window,
    sample_rate: f64,
    channels: usize,
    averagers: Vec<Averager>,
    rings: Vec<Vec<f32>>,
    filled: usize,
    frame_start: u64,
    axis: Axis,
    axis_hz: Vec<f32>,
    offset_db: f32,
    min_interval_frames: u64,
    last_emit: Option<u64>,
}

impl std::fmt::Debug for Rta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rta")
            .field("fft_length", &self.fft.len())
            .field("channels", &self.channels)
            .field("points", &self.axis_hz.len())
            .finish_non_exhaustive()
    }
}

impl Rta {
    /// `offset_db` is added to every value (0 for dBFS, the input dBV offset otherwise).
    pub fn new(cfg: &RtaConfig, channels: u16, sample_rate: u32, offset_db: f32) -> Self {
        let fs = sample_rate as f64;
        let nyquist = fs / 2.0;
        let axis = match cfg.axis {
            RtaAxis::Log {
                min_hz,
                max_hz,
                points,
            } => Axis::Log(LogAxis::new(
                min_hz.max(1.0),
                max_hz.min(nyquist).max(min_hz * 2.0),
                points.max(2),
            )),
            RtaAxis::Octave {
                fraction,
                min_hz,
                max_hz,
            } => Axis::Octave(OctaveBands::new(
                fraction.max(1),
                min_hz.max(1.0),
                max_hz.min(nyquist).max(min_hz * 2.0),
            )),
        };
        let axis_hz: Vec<f32> = match &axis {
            Axis::Log(a) => a.frequencies().iter().map(|f| *f as f32).collect(),
            Axis::Octave(b) => b.centres().iter().map(|f| *f as f32).collect(),
        };
        let ch = channels as usize;
        Self {
            fft: RealSpectrum::new(cfg.fft_length.max(16)),
            window: cfg.window,
            sample_rate: fs,
            channels: ch,
            averagers: (0..ch).map(|_| Averager::new(cfg.averaging)).collect(),
            rings: (0..ch).map(|_| vec![0.0; cfg.fft_length.max(16)]).collect(),
            filled: 0,
            frame_start: 0,
            axis,
            axis_hz,
            offset_db,
            min_interval_frames: if cfg.update_rate_hz > 0.0 {
                (fs / cfg.update_rate_hz as f64) as u64
            } else {
                0
            },
            last_emit: None,
        }
    }

    pub fn axis_hz(&self) -> &[f32] {
        &self.axis_hz
    }

    pub fn points(&self) -> usize {
        self.axis_hz.len()
    }

    /// Refresh the dB offset (input range changed).
    pub fn set_offset_db(&mut self, offset_db: f32) {
        self.offset_db = offset_db;
    }

    /// Feed a block; returns the frames completed inside it.
    pub fn push(&mut self, block: &InputBlock) -> Vec<Reading> {
        let ch = self.channels;
        let n = self.fft.len();
        let mut out = Vec::new();
        for (i, frame) in block.samples.chunks_exact(ch).enumerate() {
            if self.filled == 0 {
                self.frame_start = block.first_frame + i as u64;
            }
            for (c, v) in frame.iter().enumerate() {
                self.rings[c][self.filled] = *v;
            }
            self.filled += 1;
            if self.filled == n {
                self.filled = 0;
                if let Some(r) = self.analyze() {
                    out.push(r);
                }
            }
        }
        out
    }

    fn analyze(&mut self) -> Option<Reading> {
        let mut values = Vec::with_capacity(self.channels * self.axis_hz.len());
        for c in 0..self.channels {
            let power = self.fft.power(&self.rings[c], self.window);
            let avg = self.averagers[c].push(&power);
            let per_point: Vec<f64> = match &self.axis {
                Axis::Log(a) => a.decimate(avg, self.sample_rate),
                Axis::Octave(b) => b.band_powers(avg, self.sample_rate, self.window),
            };
            values.extend(
                per_point
                    .iter()
                    .map(|p| anecho_dsp::units::power_db(*p) as f32 + self.offset_db),
            );
        }
        let emit = match self.last_emit {
            Some(last) => self.frame_start.saturating_sub(last) >= self.min_interval_frames,
            None => true,
        };
        if !emit {
            return None;
        }
        self.last_emit = Some(self.frame_start);
        Some(Reading {
            first_frame: self.frame_start,
            values,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sine_block(hz: f64, peak: f64, frames: usize, fs: u32) -> InputBlock {
        let samples: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let v = (peak * (std::f64::consts::TAU * hz * i as f64 / fs as f64).sin()) as f32;
                [v, 0.0]
            })
            .collect();
        InputBlock {
            seq: 0,
            first_frame: 0,
            channels: 2,
            frames: frames as u32,
            samples: Arc::from(samples),
            dropped_before: 0,
            scale: anecho_device::Scale::Dbfs,
        }
    }

    #[test]
    fn bin_centred_sine_reads_its_rms_level_on_the_log_axis() {
        // 1500 Hz = bin 512 of a 16384-point FFT at 48 kHz: no scalloping.
        let cfg = RtaConfig::default();
        let mut rta = Rta::new(&cfg, 2, 48_000, 0.0);
        let readings = rta.push(&sine_block(1500.0, 0.1, 16_384, 48_000));
        assert_eq!(readings.len(), 1);
        let r = &readings[0];
        let (idx, _) = rta
            .axis_hz()
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - 1500.0)
                    .abs()
                    .partial_cmp(&(b.1 - 1500.0).abs())
                    .unwrap()
            })
            .unwrap();
        let v = r.values[idx];
        assert!(
            (v + 23.01).abs() < 0.1,
            "{v} dBFS_rms at {} Hz",
            rta.axis_hz()[idx]
        );
        // The other channel is silent.
        assert!(r.values[rta.points() + idx] < -150.0);
    }

    #[test]
    fn octave_bands_carry_the_sine_in_one_band() {
        let cfg = RtaConfig {
            axis: RtaAxis::Octave {
                fraction: 3,
                min_hz: 20.0,
                max_hz: 20_000.0,
            },
            ..Default::default()
        };
        let mut rta = Rta::new(&cfg, 2, 48_000, 0.0);
        let r = rta.push(&sine_block(1500.0, 0.1, 16_384, 48_000)).remove(0);
        let (idx, _) = rta
            .axis_hz()
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1 - 1587.4)
                    .abs()
                    .partial_cmp(&(b.1 - 1587.4).abs())
                    .unwrap()
            })
            .unwrap();
        assert!((r.values[idx] + 23.01).abs() < 0.3, "{}", r.values[idx]);
        for (i, v) in r.values[..rta.points()].iter().enumerate() {
            if i != idx {
                assert!(*v < -80.0, "band {} Hz: {v}", rta.axis_hz()[i]);
            }
        }
    }
}
