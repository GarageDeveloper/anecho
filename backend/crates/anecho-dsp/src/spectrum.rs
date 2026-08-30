//! Spectrum post-processing: averaging, logarithmic display axis, octave bands.
//!
//! Everything here operates on **per-bin power** (`fft::RealSpectrum::power`), never on dB,
//! so that averages and band sums are physically meaningful.

use crate::window::Window;

/// How successive spectra are combined.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Averaging {
    /// Each block replaces the previous one.
    None,
    /// `y ← y + (x − y)/n` once `n` blocks have been seen, running mean before that.
    Exponential { n: u32 },
    /// Running mean of the last `n` blocks (then frozen — call [`Averager::reset`]).
    Linear { n: u32 },
    /// Per-bin maximum since the last reset.
    PeakHold,
}

/// Per-channel averager of power spectra.
#[derive(Debug, Clone)]
pub struct Averager {
    mode: Averaging,
    acc: Vec<f64>,
    count: u32,
}

impl Averager {
    pub fn new(mode: Averaging) -> Self {
        Self {
            mode,
            acc: Vec::new(),
            count: 0,
        }
    }

    pub fn mode(&self) -> Averaging {
        self.mode
    }

    pub fn reset(&mut self) {
        self.acc.clear();
        self.count = 0;
    }

    /// Blocks seen since the last reset.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Feed one power spectrum; returns the current average.
    pub fn push(&mut self, power: &[f64]) -> &[f64] {
        if self.acc.len() != power.len() {
            self.acc = vec![0.0; power.len()];
            self.count = 0;
        }
        self.count = self.count.saturating_add(1);
        match self.mode {
            Averaging::None => self.acc.copy_from_slice(power),
            Averaging::Exponential { n } => {
                let k = 1.0 / (self.count.min(n.max(1)) as f64);
                for (a, p) in self.acc.iter_mut().zip(power) {
                    *a += (p - *a) * k;
                }
            }
            Averaging::Linear { n } => {
                let n = n.max(1);
                if self.count <= n {
                    let k = 1.0 / self.count as f64;
                    for (a, p) in self.acc.iter_mut().zip(power) {
                        *a += (p - *a) * k;
                    }
                }
            }
            Averaging::PeakHold => {
                if self.count == 1 {
                    self.acc.copy_from_slice(power);
                } else {
                    for (a, p) in self.acc.iter_mut().zip(power) {
                        *a = a.max(*p);
                    }
                }
            }
        }
        &self.acc
    }

    /// True once a linear average has collected its `n` blocks.
    pub fn is_complete(&self) -> bool {
        match self.mode {
            Averaging::Linear { n } => self.count >= n.max(1),
            _ => false,
        }
    }
}

/// Logarithmically spaced display frequencies.
#[derive(Debug, Clone, PartialEq)]
pub struct LogAxis {
    points: Vec<f64>,
}

impl LogAxis {
    /// `points` frequencies from `min_hz` to `max_hz` inclusive, geometric spacing.
    pub fn new(min_hz: f64, max_hz: f64, points: usize) -> Self {
        assert!(min_hz > 0.0 && max_hz > min_hz && points >= 2);
        let ratio = (max_hz / min_hz).ln() / (points - 1) as f64;
        Self {
            points: (0..points)
                .map(|i| min_hz * (i as f64 * ratio).exp())
                .collect(),
        }
    }

    pub fn frequencies(&self) -> &[f64] {
        &self.points
    }

    /// Reduce per-bin values to the axis: for each point, the **maximum** over the bins whose
    /// centre falls in the point's cell (geometric midpoints to its neighbours); when a cell
    /// covers no bin centre, the value is linearly interpolated between the two surrounding
    /// bins. Works on any per-bin quantity (power, PSD, magnitude) — the caller converts to dB.
    pub fn decimate(&self, bins: &[f64], sample_rate: f64) -> Vec<f64> {
        let fft_len = (bins.len() - 1) * 2;
        let bin_hz = sample_rate / fft_len as f64;
        let n = self.points.len();
        (0..n)
            .map(|i| {
                let f = self.points[i];
                let lo = if i == 0 {
                    f
                } else {
                    (self.points[i - 1] * f).sqrt()
                };
                let hi = if i == n - 1 {
                    f
                } else {
                    (f * self.points[i + 1]).sqrt()
                };
                let k_lo = (lo / bin_hz).ceil() as usize;
                let k_hi = (hi / bin_hz).floor() as usize;
                if k_lo <= k_hi && k_hi < bins.len() {
                    bins[k_lo..=k_hi].iter().cloned().fold(f64::MIN, f64::max)
                } else {
                    let pos = f / bin_hz;
                    let k = (pos.floor() as usize).min(bins.len().saturating_sub(2));
                    let t = (pos - k as f64).clamp(0.0, 1.0);
                    bins[k] * (1.0 - t) + bins[k + 1] * t
                }
            })
            .collect()
    }
}

/// Fractional-octave bands, base 2, centred on 1 kHz: `f_c(k) = 1000 · 2^(k / fraction)`,
/// edges `f_c · 2^(∓1 / (2·fraction))`.
#[derive(Debug, Clone, PartialEq)]
pub struct OctaveBands {
    fraction: u32,
    centres: Vec<f64>,
}

impl OctaveBands {
    pub fn new(fraction: u32, min_hz: f64, max_hz: f64) -> Self {
        assert!(fraction >= 1 && min_hz > 0.0 && max_hz > min_hz);
        let f = fraction as f64;
        let k_lo = (f * (min_hz / 1000.0).log2()).ceil() as i64;
        let k_hi = (f * (max_hz / 1000.0).log2()).floor() as i64;
        Self {
            fraction,
            centres: (k_lo..=k_hi)
                .map(|k| 1000.0 * 2f64.powf(k as f64 / f))
                .collect(),
        }
    }

    pub fn fraction(&self) -> u32 {
        self.fraction
    }

    pub fn centres(&self) -> &[f64] {
        &self.centres
    }

    /// `(lower, upper)` edges of band `i`.
    pub fn edges(&self, i: usize) -> (f64, f64) {
        let half = 2f64.powf(1.0 / (2.0 * self.fraction as f64));
        (self.centres[i] / half, self.centres[i] * half)
    }

    /// Power per band: `Σ P_k / ENBW` over the bins whose centre lies inside the band; bins on
    /// an edge are split by the fraction of their width inside the band.
    pub fn band_powers(&self, power_bins: &[f64], sample_rate: f64, window: Window) -> Vec<f64> {
        let fft_len = (power_bins.len() - 1) * 2;
        let bin_hz = sample_rate / fft_len as f64;
        let enbw = window.enbw_bins();
        (0..self.centres.len())
            .map(|i| {
                let (lo, hi) = self.edges(i);
                band_power_between(power_bins, bin_hz, lo, hi) / enbw
            })
            .collect()
    }
}

/// Sum of per-bin power between two frequencies, splitting edge bins proportionally.
/// Not ENBW-corrected — see [`band_power`].
fn band_power_between(power_bins: &[f64], bin_hz: f64, lo: f64, hi: f64) -> f64 {
    let mut sum = 0.0;
    for (k, p) in power_bins.iter().enumerate() {
        let b_lo = (k as f64 - 0.5) * bin_hz;
        let b_hi = (k as f64 + 0.5) * bin_hz;
        let overlap = (b_hi.min(hi) - b_lo.max(lo)).max(0.0);
        if overlap > 0.0 {
            sum += p * overlap / bin_hz;
        }
    }
    sum
}

/// Broadband power between `lo` and `hi` Hz: `Σ P_k / ENBW` (edge bins split proportionally).
/// For a sine this is its RMS² (its main lobe must fit inside the band); for noise it is the
/// noise power in the band.
pub fn band_power(power_bins: &[f64], sample_rate: f64, window: Window, lo: f64, hi: f64) -> f64 {
    let fft_len = (power_bins.len() - 1) * 2;
    let bin_hz = sample_rate / fft_len as f64;
    band_power_between(power_bins, bin_hz, lo, hi) / window.enbw_bins()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averagers() {
        let mut a = Averager::new(Averaging::Linear { n: 2 });
        a.push(&[2.0]);
        assert_eq!(a.push(&[4.0]), &[3.0]);
        assert!(a.is_complete());
        assert_eq!(a.push(&[100.0]), &[3.0], "frozen after n");
        let mut e = Averager::new(Averaging::Exponential { n: 4 });
        e.push(&[1.0]);
        e.push(&[1.0]);
        e.push(&[1.0]);
        e.push(&[1.0]);
        assert!((e.push(&[5.0])[0] - 2.0).abs() < 1e-12);
        let mut p = Averager::new(Averaging::PeakHold);
        p.push(&[1.0, 5.0]);
        assert_eq!(p.push(&[3.0, 2.0]), &[3.0, 5.0]);
    }

    #[test]
    fn log_axis_and_decimation() {
        let axis = LogAxis::new(20.0, 20_000.0, 4);
        let f = axis.frequencies();
        assert!((f[0] - 20.0).abs() < 1e-9 && (f[3] - 20_000.0).abs() < 1e-6);
        assert!((f[1] / f[0] - f[2] / f[1]).abs() < 1e-9);
        // 1 kHz peak in bin 100 of a 4096-point FFT at 48 kHz survives decimation.
        let mut bins = vec![0.0; 2049];
        bins[100] = 1.0;
        let axis = LogAxis::new(20.0, 20_000.0, 200);
        let d = axis.decimate(&bins, 48_000.0);
        assert!((d.iter().cloned().fold(0.0, f64::max) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn octave_bands_base_2() {
        let b = OctaveBands::new(3, 20.0, 20_000.0);
        assert!(b.centres().iter().any(|c| (c - 1000.0).abs() < 1e-9));
        let (lo, hi) = b.edges(
            b.centres()
                .iter()
                .position(|c| (c - 1000.0).abs() < 1e-9)
                .unwrap(),
        );
        assert!((lo - 890.9).abs() < 0.1 && (hi - 1122.5).abs() < 0.1);
        // 1000·2^(-16/3) = 24.80 Hz: the 19.69 Hz band lies below min_hz and is excluded.
        assert!((b.centres()[0] - 24.803).abs() < 0.01, "{}", b.centres()[0]);
    }
}
