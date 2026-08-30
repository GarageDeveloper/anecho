//! Real FFT and calibrated magnitude spectra.
//!
//! Definitions, for a block of `N` samples `x[n]` and window `w[n]` with coherent gain
//! `CG = Σw/N` and noise bandwidth `ENBW` (bins):
//!
//! - `X_k = Σ x[n]·w[n]·e^{−j2πkn/N}`, `k = 0..=N/2`.
//! - Per-bin **RMS amplitude** (single-sided, linear full scale):
//!   `A_k = |X_k| · s_k / (N·CG) / √2` with `s_k = 1` for DC and Nyquist, `2` otherwise.
//!   A full-scale sine (peak 1) exactly on bin `k` gives `A_k = 1/√2`.
//! - Per-bin power `P_k = A_k²`. Broadband power over a set of bins is `Σ P_k / ENBW`
//!   (exact for a sine's main lobe and for noise — see `spectrum::band_power`).
//! - PSD (per Hz): `S_k = P_k / (ENBW · fs / N)`.
//!
//! Level helpers: [`db_peak`] reports a bin as the peak level of the sine it would represent
//! (`20·log10(A_k·√2)`, full-scale sine = 0 dBFS — the crate convention), [`db_rms`] reports
//! `20·log10(A_k)`.

use crate::window::Window;
use num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A real-to-complex FFT of a fixed length, with reusable scratch buffers.
pub struct RealSpectrum {
    len: usize,
    plan: Arc<dyn RealToComplex<f64>>,
    input: Vec<f64>,
    output: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
}

impl std::fmt::Debug for RealSpectrum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealSpectrum")
            .field("len", &self.len)
            .finish()
    }
}

type PlanCache = Mutex<HashMap<usize, Arc<dyn RealToComplex<f64>>>>;

fn plan(len: usize) -> Arc<dyn RealToComplex<f64>> {
    static CACHE: OnceLock<PlanCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    guard
        .entry(len)
        .or_insert_with(|| RealFftPlanner::<f64>::new().plan_fft_forward(len))
        .clone()
}

impl RealSpectrum {
    pub fn new(len: usize) -> Self {
        assert!(len >= 2, "FFT length must be at least 2");
        let plan = plan(len);
        Self {
            len,
            input: vec![0.0; len],
            output: vec![Complex::new(0.0, 0.0); len / 2 + 1],
            scratch: vec![Complex::new(0.0, 0.0); plan.get_scratch_len()],
            plan,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of bins (`N/2 + 1`).
    pub fn bins(&self) -> usize {
        self.len / 2 + 1
    }

    /// Frequency of bin `k` at the given sample rate.
    pub fn bin_hz(&self, k: usize, sample_rate: f64) -> f64 {
        k as f64 * sample_rate / self.len as f64
    }

    /// Raw complex spectrum of the windowed block (no scaling).
    pub fn transform(&mut self, samples: &[f32], window: Window) -> &[Complex<f64>] {
        assert_eq!(
            samples.len(),
            self.len,
            "block length must match the FFT length"
        );
        let w = window.samples(self.len);
        for ((i, s), wn) in self.input.iter_mut().zip(samples).zip(w.iter()) {
            *i = *s as f64 * wn;
        }
        self.plan
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .expect("buffer lengths are fixed at construction");
        &self.output
    }

    /// Per-bin RMS amplitudes (see module docs).
    pub fn magnitude(&mut self, samples: &[f32], window: Window) -> Vec<f64> {
        let n = self.len as f64;
        let cg = window.coherent_gain();
        let last = self.bins() - 1;
        self.transform(samples, window)
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let single_sided = if k == 0 || k == last { 1.0 } else { 2.0 };
                c.norm() * single_sided / (n * cg) / std::f64::consts::SQRT_2
            })
            .collect()
    }

    /// Per-bin power `A_k²`.
    pub fn power(&mut self, samples: &[f32], window: Window) -> Vec<f64> {
        self.magnitude(samples, window)
            .into_iter()
            .map(|a| a * a)
            .collect()
    }
}

/// Convenience: one-shot magnitude spectrum (plans are cached, buffers are not).
pub fn magnitude_spectrum(samples: &[f32], window: Window) -> Vec<f64> {
    RealSpectrum::new(samples.len()).magnitude(samples, window)
}

/// Power spectral density per Hz, ENBW-corrected: `P_k / (ENBW · fs / N)`.
pub fn psd(samples: &[f32], window: Window, sample_rate: f64) -> Vec<f64> {
    let n = samples.len() as f64;
    let bin_hz = sample_rate / n;
    let enbw_hz = window.enbw_bins() * bin_hz;
    RealSpectrum::new(samples.len())
        .power(samples, window)
        .into_iter()
        .map(|p| p / enbw_hz)
        .collect()
}

/// Peak-convention level of a bin: `20·log10(A_rms·√2)`; a full-scale sine reads 0 dBFS.
pub fn db_peak(rms_amplitude: f64) -> f64 {
    crate::units::db(rms_amplitude * std::f64::consts::SQRT_2)
}

/// RMS level of a bin: `20·log10(A_rms)`; a full-scale sine reads −3.01 dBFS_rms.
pub fn db_rms(rms_amplitude: f64) -> f64 {
    crate::units::db(rms_amplitude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(n: usize, cycles: f64, peak: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (peak * (std::f64::consts::TAU * cycles * i as f64 / n as f64).sin()) as f32)
            .collect()
    }

    #[test]
    fn full_scale_sine_on_a_bin_reads_0_dbfs_with_every_window() {
        let n = 4096;
        let x = sine(n, 100.0, 1.0);
        for w in [
            Window::Rectangular,
            Window::Hann,
            Window::BlackmanHarris4,
            Window::BlackmanHarris7,
            Window::FlatTop,
        ] {
            let m = magnitude_spectrum(&x, w);
            let peak = db_peak(m[100]);
            assert!(peak.abs() < 0.01, "{w:?}: {peak} dBFS");
            assert!((db_rms(m[100]) + 3.0103).abs() < 0.01);
        }
    }

    #[test]
    fn flat_top_holds_amplitude_between_bins() {
        let n = 4096;
        let x = sine(n, 100.5, 0.5);
        let m = magnitude_spectrum(&x, Window::FlatTop);
        let peak = m.iter().cloned().fold(0.0, f64::max);
        assert!((db_peak(peak) + 6.0206).abs() < 0.02, "{}", db_peak(peak));
    }

    #[test]
    fn white_noise_psd_is_flat() {
        // Uniform noise in [-1, 1): variance 1/3, single-sided PSD = (1/3) / (fs/2).
        let n = 1 << 16;
        let fs = 48_000.0;
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let x: Vec<f32> = (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
            })
            .collect();
        let expected = (1.0 / 3.0) / (fs / 2.0);
        for w in [Window::Rectangular, Window::Hann, Window::BlackmanHarris7] {
            let p = psd(&x, w, fs);
            // Average the band 1 kHz–20 kHz to beat the per-bin variance.
            let lo = (1000.0 * n as f64 / fs) as usize;
            let hi = (20_000.0 * n as f64 / fs) as usize;
            let mean = p[lo..hi].iter().sum::<f64>() / (hi - lo) as f64;
            let err_db = 10.0 * (mean / expected).log10();
            assert!(err_db.abs() < 0.2, "{w:?}: {err_db} dB");
        }
    }
}
