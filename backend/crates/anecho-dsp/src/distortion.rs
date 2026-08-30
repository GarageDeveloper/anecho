//! Distortion analysis on stationary signals: THD, THD+N, IMD (SMPTE, CCIF).
//!
//! All quantities derive from one calibrated power spectrum (`fft::RealSpectrum::power`)
//! and the window's noise bandwidth `ENBW`:
//!
//! - Power of a spectral line at bin `k`: `P = Σ_{|j−k| ≤ m} P_j / ENBW` with
//!   `m = Window::main_lobe_bins()` (rectangular 1, Hann 2, BH4 4, BH7 7, flat-top 5).
//! - Fundamental `f0`: the highest bin in the analysis band (or within ±5 % of the hint),
//!   refined by parabolic interpolation of the dB magnitudes of the three bins around it:
//!   `δ = ½ (α − γ) / (α − 2β + γ)` bins, `f0 = (k + δ)·fs/N`.
//! - `THD = √(Σ_{n=2..N} P_n) / √P_1`, harmonics at bins `round(n·f0·N/fs)`, only those
//!   below the band's upper edge.
//! - `THD+N = √(P_band − P_1) / √P_1` where `P_band = Σ P_k / ENBW` over the analysis band
//!   (default 20 Hz to min(20 kHz, 0.45·fs)).
//! - Noise floor: median per-bin level of the band, in the same units as the bins (dBFS_rms
//!   per bin), a display figure rather than a calibrated noise power (use `fft::psd` for that).
//! - SMPTE IMD (60 Hz + 7 kHz, 4:1): `√(Σ_{n=1..4} P(f_h ± n·f_l)) / √P(f_h)`.
//! - CCIF IMD (19 kHz + 20 kHz, 1:1): `√(P(f2−f1) + P(2f1−f2) + P(2f2−f1)) / √(P(f1) + P(f2))`
//!   (second- and third-order difference products, as REW reports "IMD CCIF").
//!
//! Window choice: THD+N subtracts the fundamental's main lobe from the band power, so
//! sidelobe leakage of a tone that is not bin-centred counts as "noise" — with Hann it can
//! reach ~1.7 % (see the golden snapshot), with the default Blackman-Harris 7-term it is
//! below 0.001 %. Use the default (or flat-top) for distortion figures; Hann is for display.
//!
//! Levels: `fundamental_level_db` follows the crate convention (peak dBFS of the fundamental:
//! `10·log10(2·P_1)`); harmonics are relative to the fundamental in dB.

use crate::fft::RealSpectrum;
use crate::spectrum::band_power;
use crate::units::{db, power_db};
use crate::window::Window;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Harmonic {
    pub n: u32,
    pub hz: f64,
    /// Level relative to the fundamental, dB.
    pub level_db_rel: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DistortionResult {
    pub fundamental_hz: f64,
    /// Peak level of the fundamental, dBFS (crate convention).
    pub fundamental_level_db: f64,
    pub thd_pct: f64,
    pub thd_db: f64,
    pub thd_n_pct: f64,
    pub thd_n_db: f64,
    pub harmonics: Vec<Harmonic>,
    /// Median per-bin level in the band, dBFS_rms per bin.
    pub noise_floor_db: f64,
    /// Analysis band actually used, Hz.
    pub band_hz: (f64, f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThdOptions {
    pub window: Window,
    /// Highest harmonic order included (default 9).
    pub max_harmonic: u32,
    /// Expected fundamental; the search is restricted to ±5 % around it.
    pub fundamental_hint: Option<f64>,
    /// Analysis band; `None` = 20 Hz to min(20 kHz, 0.45·fs).
    pub band_hz: Option<(f64, f64)>,
}

impl Default for ThdOptions {
    fn default() -> Self {
        Self {
            window: Window::BlackmanHarris7,
            max_harmonic: 9,
            fundamental_hint: None,
            band_hz: None,
        }
    }
}

/// Locate the fundamental in a **power** spectrum: `(hz, peak level dBFS, bin index)`.
pub fn find_fundamental(
    power: &[f64],
    sample_rate: f64,
    hint: Option<f64>,
    band: (f64, f64),
) -> (f64, f64, usize) {
    let fft_len = (power.len() - 1) * 2;
    let bin_hz = sample_rate / fft_len as f64;
    let (lo, hi) = match hint {
        Some(h) => (h * 0.95, h * 1.05),
        None => band,
    };
    let k_lo = ((lo / bin_hz).floor() as usize).max(1);
    let k_hi = ((hi / bin_hz).ceil() as usize).min(power.len() - 2);
    let k = (k_lo..=k_hi)
        .max_by(|a, b| power[*a].partial_cmp(&power[*b]).unwrap())
        .unwrap_or(k_lo);
    let (a, b, c) = (
        power_db(power[k - 1]),
        power_db(power[k]),
        power_db(power[k + 1]),
    );
    let denom = a - 2.0 * b + c;
    let delta = if denom.abs() > 1e-12 {
        0.5 * (a - c) / denom
    } else {
        0.0
    };
    let delta = delta.clamp(-0.5, 0.5);
    let peak_db = b - 0.25 * (a - c) * delta;
    // `b` is 10·log10(P) of an RMS power: peak convention adds 3.01 dB.
    (
        (k as f64 + delta) * bin_hz,
        peak_db + 10.0 * 2f64.log10(),
        k,
    )
}

fn line_power(power: &[f64], k: usize, window: Window) -> f64 {
    let m = window.main_lobe_bins();
    let lo = k.saturating_sub(m);
    let hi = (k + m).min(power.len() - 1);
    power[lo..=hi].iter().sum::<f64>() / window.enbw_bins()
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

#[derive(Debug)]
pub struct Thd;

impl Thd {
    /// Analyse one block (length = FFT length).
    pub fn analyze(samples: &[f32], sample_rate: f64, opts: &ThdOptions) -> DistortionResult {
        let mut fft = RealSpectrum::new(samples.len());
        let power = fft.power(samples, opts.window);
        Self::analyze_power(&power, sample_rate, opts)
    }

    /// Same, from an already computed (possibly averaged) power spectrum.
    pub fn analyze_power(power: &[f64], sample_rate: f64, opts: &ThdOptions) -> DistortionResult {
        let fft_len = (power.len() - 1) * 2;
        let bin_hz = sample_rate / fft_len as f64;
        let band = opts
            .band_hz
            .unwrap_or((20.0, (20_000.0f64).min(0.45 * sample_rate)));
        let (f0, level_db, k0) = find_fundamental(power, sample_rate, opts.fundamental_hint, band);
        let p1 = line_power(power, k0, opts.window);

        let mut harmonics = Vec::new();
        let mut p_harm = 0.0;
        for n in 2..=opts.max_harmonic.max(2) {
            let hz = n as f64 * f0;
            if hz >= band.1 {
                break;
            }
            let k = (hz / bin_hz).round() as usize;
            if k + opts.window.main_lobe_bins() >= power.len() {
                break;
            }
            let p = line_power(power, k, opts.window);
            p_harm += p;
            harmonics.push(Harmonic {
                n,
                hz,
                level_db_rel: power_db(p / p1),
            });
        }
        let p_band = band_power(power, sample_rate, opts.window, band.0, band.1);
        let p_noise_dist = (p_band - p1).max(0.0);
        let thd = (p_harm / p1).sqrt();
        let thd_n = (p_noise_dist / p1).sqrt();

        let k_lo = (band.0 / bin_hz).ceil() as usize;
        let k_hi = ((band.1 / bin_hz).floor() as usize).min(power.len() - 1);
        let floor = median(power[k_lo..=k_hi].iter().map(|p| power_db(*p)).collect());

        DistortionResult {
            fundamental_hz: f0,
            fundamental_level_db: level_db,
            thd_pct: thd * 100.0,
            thd_db: db(thd),
            thd_n_pct: thd_n * 100.0,
            thd_n_db: db(thd_n),
            harmonics,
            noise_floor_db: floor,
            band_hz: band,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ImdResult {
    pub imd_pct: f64,
    pub imd_db: f64,
    /// `(hz, level dB relative to the reference tone(s))` of each product considered.
    pub products: Vec<(f64, f64)>,
}

#[derive(Debug)]
pub struct Imd;

impl Imd {
    /// SMPTE/DIN: low tone `f_low` (60 Hz SMPTE, 250 Hz DIN) at 4:1 with `f_high` (7 kHz).
    /// Products: `f_high ± n·f_low`, n = 1..=4, relative to the high tone.
    pub fn smpte(
        samples: &[f32],
        sample_rate: f64,
        window: Window,
        f_low: f64,
        f_high: f64,
    ) -> ImdResult {
        let mut fft = RealSpectrum::new(samples.len());
        let power = fft.power(samples, window);
        let bin_hz = sample_rate / samples.len() as f64;
        let k = |hz: f64| (hz / bin_hz).round() as usize;
        let p_ref = line_power(&power, k(f_high), window);
        let mut p_sum = 0.0;
        let mut products = Vec::new();
        for n in 1..=4 {
            for hz in [f_high - n as f64 * f_low, f_high + n as f64 * f_low] {
                let p = line_power(&power, k(hz), window);
                p_sum += p;
                products.push((hz, power_db(p / p_ref)));
            }
        }
        let imd = (p_sum / p_ref).sqrt();
        ImdResult {
            imd_pct: imd * 100.0,
            imd_db: db(imd),
            products,
        }
    }

    /// CCIF/ITU-R: two equal tones `f1`, `f2` (19 kHz + 20 kHz). Products: `f2−f1`,
    /// `2f1−f2`, `2f2−f1`, relative to the sum of both tones.
    pub fn ccif(samples: &[f32], sample_rate: f64, window: Window, f1: f64, f2: f64) -> ImdResult {
        let mut fft = RealSpectrum::new(samples.len());
        let power = fft.power(samples, window);
        let bin_hz = sample_rate / samples.len() as f64;
        let k = |hz: f64| (hz / bin_hz).round() as usize;
        let p_ref = line_power(&power, k(f1), window) + line_power(&power, k(f2), window);
        let mut p_sum = 0.0;
        let mut products = Vec::new();
        for hz in [f2 - f1, 2.0 * f1 - f2, 2.0 * f2 - f1] {
            if hz <= 0.0 || hz >= sample_rate / 2.0 {
                continue;
            }
            let p = line_power(&power, k(hz), window);
            p_sum += p;
            products.push((hz, power_db(p / p_ref)));
        }
        let imd = (p_sum / p_ref).sqrt();
        ImdResult {
            imd_pct: imd * 100.0,
            imd_db: db(imd),
            products,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, fs: f64, hz: f64, peak: f64, out: &mut [f64]) {
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o += peak * (std::f64::consts::TAU * hz * i as f64 / fs).sin();
        }
    }

    #[test]
    fn thd_of_known_harmonics() {
        let n = 32_768;
        let fs = 48_000.0;
        let mut x = vec![0.0; n];
        tone(n, fs, 1000.0, 0.5, &mut x);
        tone(n, fs, 2000.0, 0.5 * 1e-3, &mut x); // H2 at -60 dB
        tone(n, fs, 3000.0, 0.5 * 10f64.powf(-70.0 / 20.0), &mut x); // H3 at -70 dB
        let x: Vec<f32> = x.iter().map(|v| *v as f32).collect();
        let r = Thd::analyze(&x, fs, &ThdOptions::default());
        let expected = (1e-6f64 + 1e-7).sqrt() * 100.0; // 0.10488 %
        assert!(
            (r.fundamental_hz - 1000.0).abs() < 0.5,
            "{}",
            r.fundamental_hz
        );
        assert!(
            (r.fundamental_level_db + 6.0206).abs() < 0.02,
            "{}",
            r.fundamental_level_db
        );
        assert!(
            (r.thd_pct - expected).abs() < 0.001,
            "thd {} vs {expected}",
            r.thd_pct
        );
        assert!((r.harmonics[0].level_db_rel + 60.0).abs() < 0.05);
        assert!((r.harmonics[1].level_db_rel + 70.0).abs() < 0.05);
        // No noise: THD+N ≈ THD.
        assert!(
            (r.thd_n_pct - expected).abs() < 0.002,
            "thd+n {}",
            r.thd_n_pct
        );
    }

    #[test]
    fn smpte_imd_of_known_modulation() {
        // Amplitude-modulate the 7 kHz tone by 1 % (sidebands at ±60 Hz, each -46.02 dB):
        // IMD = sqrt(2 * (0.005)^2) = 0.707 %.
        let n = 32_768;
        let fs = 48_000.0;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / fs;
                let low = 0.4 * (std::f64::consts::TAU * 60.0 * t).sin();
                let high = 0.1
                    * (1.0 + 0.01 * (std::f64::consts::TAU * 60.0 * t).cos())
                    * (std::f64::consts::TAU * 7000.0 * t).sin();
                (low + high) as f32
            })
            .collect();
        let r = Imd::smpte(&x, fs, Window::BlackmanHarris7, 60.0, 7000.0);
        let expected = 100.0 / 2f64.sqrt() / 100.0;
        assert!((r.imd_pct - expected).abs() < 0.01, "{}", r.imd_pct);
    }
}
