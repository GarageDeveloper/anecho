//! Test signals driving a device's outputs: a real-time adapter over
//! [`anecho_dsp::generator::SignalGen`].
//!
//! [`GeneratorSpec`] is what the API asks for (signal, level in peak dBFS or dBV RMS, which
//! output channels to drive); the engine resolves a dBV level against the device's output
//! ranges and calibration into a digital peak level before building the [`Generator`].

use anecho_device::OutputSource;
pub use anecho_dsp::generator::{Level, Signal, SignalGen};

/// Requested level of a generated signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GenLevel {
    /// Peak level in dBFS (full-scale sine = 0 dBFS).
    PeakDbfs(f64),
    /// RMS level in dBV; needs a factory-calibrated device (the engine picks the output
    /// range and the digital level).
    DbvRms(f64),
}

/// A generator request as it arrives from the API.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorSpec {
    pub signal: Signal,
    pub level: GenLevel,
    /// Output channels to drive (indices in the session's output channel list); empty = all.
    pub output_channels: Vec<u16>,
}

/// Crest factor of a signal, measured over one second at unit peak: `(peak, rms)` with
/// `crest_db = 20·log10(peak / rms)`. Deterministic (the generators are).
pub fn crest_factor_db(signal: &Signal, sample_rate: u32) -> f64 {
    let mut g = SignalGen::new(signal.clone(), sample_rate, Level::Linear(1.0));
    let mut buf = vec![0f32; sample_rate as usize];
    g.fill(&mut buf);
    let peak = buf.iter().fold(0f64, |m, v| m.max((*v as f64).abs()));
    let rms = (buf.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / buf.len() as f64).sqrt();
    if peak <= 0.0 || rms <= 0.0 {
        return 3.0103;
    }
    20.0 * (peak / rms).log10()
}

/// Largest block a device callback is expected to ask for, in frames. `fill` never
/// allocates below this; above it the mono buffer grows once (logged).
const PREALLOCATED_FRAMES: usize = 1 << 16;

/// Real-time output source: one mono generator copied to the enabled channels.
pub struct Generator {
    generator: SignalGen,
    mono: Vec<f32>,
    /// Channel mask in the stream's channel order; empty = every channel.
    mask: Vec<bool>,
}

impl std::fmt::Debug for Generator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Generator")
            .field("mask", &self.mask)
            .finish_non_exhaustive()
    }
}

impl Generator {
    /// `mask[i]` tells whether output channel `i` of the stream is driven.
    pub fn new(signal: Signal, sample_rate: u32, level: Level, mask: Vec<bool>) -> Self {
        Self {
            generator: SignalGen::new(signal, sample_rate, level),
            mono: vec![0.0; PREALLOCATED_FRAMES],
            mask,
        }
    }
}

impl OutputSource for Generator {
    fn fill(&mut self, buf: &mut [f32], channels: u16, _sample_rate: u32) {
        let ch = channels as usize;
        if ch == 0 {
            return;
        }
        let frames = buf.len() / ch;
        if frames > self.mono.len() {
            log::warn!("generator: growing the mono buffer to {frames} frames");
            self.mono.resize(frames, 0.0);
        }
        self.generator.fill(&mut self.mono[..frames]);
        for (frame, v) in buf.chunks_exact_mut(ch).zip(&self.mono) {
            for (c, out) in frame.iter_mut().enumerate() {
                *out = if self.mask.is_empty() || self.mask.get(c).copied().unwrap_or(false) {
                    *v
                } else {
                    0.0
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_crest_is_3_db() {
        let c = crest_factor_db(&Signal::Sine { hz: 1000.0 }, 48_000);
        assert!((c - 3.0103).abs() < 0.01, "{c}");
    }

    #[test]
    fn mask_drives_only_selected_channels() {
        let mut g = Generator::new(
            Signal::Sine { hz: 1000.0 },
            48_000,
            Level::Dbfs(0.0),
            vec![false, true],
        );
        let mut buf = vec![0f32; 96];
        g.fill(&mut buf, 2, 48_000);
        assert!(buf.iter().step_by(2).all(|v| *v == 0.0));
        assert!(buf.iter().skip(1).step_by(2).any(|v| v.abs() > 0.5));
    }
}
