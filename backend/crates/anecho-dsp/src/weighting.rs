//! Frequency weightings A, C, Z (IEC 61672-1).
//!
//! Analogue prototypes, with `f1 = 20.598997 Hz`, `f2 = 107.65265 Hz`, `f3 = 737.86223 Hz`,
//! `f4 = 12194.217 Hz` (`ω_i = 2π f_i`):
//!
//! - `H_A(s) = K_A · s⁴ / ((s + ω1)² (s + ω2) (s + ω3) (s + ω4)²)`
//! - `H_C(s) = K_C · s² / ((s + ω1)² (s + ω4)²)`
//! - `H_Z(s) = 1`
//!
//! Closed-form magnitudes used for checking (and for frequency-domain weighting):
//! `R_A(f) = f4² f⁴ / ((f² + f1²) √((f² + f2²)(f² + f3²)) (f² + f4²))`, `A(f) = 20·log10 R_A + 2.00 dB`;
//! `R_C(f) = f4² f² / ((f² + f1²)(f² + f4²))`, `C(f) = 20·log10 R_C + 0.06 dB`.
//!
//! Time-domain filters are cascades of second-order sections obtained by the bilinear
//! transform `s = K (1 − z⁻¹)/(1 + z⁻¹)` of the factored prototype
//! (A: `s²/(s+ω1)²`, `s²/((s+ω2)(s+ω3))`, `1/(s+ω4)²`; C: `s²/(s+ω1)²`, `1/(s+ω4)²`).
//! All sections are pre-warped at the 1 kHz reference, `K = ω_r / tan(ω_r / 2fs)`, and the
//! cascade is normalised numerically to 0 dB at 1 kHz. A bilinear cascade cannot match the
//! analogue curve everywhere: the residual at 48 kHz stays within the IEC 61672 class 1
//! tolerances (see the test), with the largest deviation near the 12.2 kHz poles. For exact
//! weighting of a spectrum use [`Weighting::db_at`] in the frequency domain instead.

use crate::units::db;
use std::f64::consts::TAU;

const F1: f64 = 20.598_997;
const F2: f64 = 107.652_65;
const F3: f64 = 737.862_23;
const F4: f64 = 12_194.217;
/// Reference frequency (0 dB) and bilinear pre-warp point.
const F_REF: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Weighting {
    A,
    C,
    Z,
}

impl Weighting {
    /// Analytic weighting in dB at frequency `hz` (0 dB at 1 kHz by definition).
    pub fn db_at(self, hz: f64) -> f64 {
        let f2 = hz * hz;
        match self {
            Weighting::A => {
                let r = F4 * F4 * f2 * f2
                    / ((f2 + F1 * F1) * ((f2 + F2 * F2) * (f2 + F3 * F3)).sqrt() * (f2 + F4 * F4));
                db(r) + 2.0
            }
            Weighting::C => {
                let r = F4 * F4 * f2 / ((f2 + F1 * F1) * (f2 + F4 * F4));
                db(r) + 0.06
            }
            Weighting::Z => 0.0,
        }
    }
}

/// Direct-form II transposed biquad.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// Bilinear transform of `(B2 s² + B1 s + B0) / (A2 s² + A1 s + A0)`, pre-warped so that
    /// the analogue frequency `warp_hz` maps exactly.
    fn from_analog(b: [f64; 3], a: [f64; 3], fs: f64, warp_hz: f64) -> Self {
        let wp = TAU * warp_hz;
        let k = wp / (wp / (2.0 * fs)).tan();
        let k2 = k * k;
        let (b2, b1, b0) = (b[0], b[1], b[2]);
        let (a2, a1, a0) = (a[0], a[1], a[2]);
        let d = a2 * k2 + a1 * k + a0;
        Self {
            b0: (b2 * k2 + b1 * k + b0) / d,
            b1: (2.0 * b0 - 2.0 * b2 * k2) / d,
            b2: (b2 * k2 - b1 * k + b0) / d,
            a1: (2.0 * a0 - 2.0 * a2 * k2) / d,
            a2: (a2 * k2 - a1 * k + a0) / d,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn magnitude_at(&self, hz: f64, fs: f64) -> f64 {
        let w = TAU * hz / fs;
        let z1 = num_complex::Complex::from_polar(1.0, -w);
        let z2 = z1 * z1;
        let num = self.b0 + self.b1 * z1 + self.b2 * z2;
        let den = 1.0 + self.a1 * z1 + self.a2 * z2;
        (num / den).norm()
    }
}

/// Time-domain weighting filter.
#[derive(Debug, Clone)]
pub struct WeightingFilter {
    sections: Vec<Biquad>,
    gain: f64,
}

impl WeightingFilter {
    pub fn new(weighting: Weighting, sample_rate: u32) -> Self {
        let fs = sample_rate as f64;
        let (w1, w2, w3, w4) = (TAU * F1, TAU * F2, TAU * F3, TAU * F4);
        let sections = match weighting {
            Weighting::A => vec![
                Biquad::from_analog([1.0, 0.0, 0.0], [1.0, 2.0 * w1, w1 * w1], fs, F_REF),
                Biquad::from_analog([1.0, 0.0, 0.0], [1.0, w2 + w3, w2 * w3], fs, F_REF),
                Biquad::from_analog([0.0, 0.0, 1.0], [1.0, 2.0 * w4, w4 * w4], fs, F_REF),
            ],
            Weighting::C => vec![
                Biquad::from_analog([1.0, 0.0, 0.0], [1.0, 2.0 * w1, w1 * w1], fs, F_REF),
                Biquad::from_analog([0.0, 0.0, 1.0], [1.0, 2.0 * w4, w4 * w4], fs, F_REF),
            ],
            Weighting::Z => vec![],
        };
        let raw: f64 = sections
            .iter()
            .map(|s| s.magnitude_at(1000.0, fs))
            .product();
        Self {
            sections,
            gain: if raw > 0.0 { 1.0 / raw } else { 1.0 },
        }
    }

    /// Magnitude response of the digital filter, dB.
    pub fn db_at(&self, hz: f64, sample_rate: u32) -> f64 {
        let fs = sample_rate as f64;
        db(self.gain
            * self
                .sections
                .iter()
                .map(|s| s.magnitude_at(hz, fs))
                .product::<f64>())
    }

    pub fn reset(&mut self) {
        for s in &mut self.sections {
            s.z1 = 0.0;
            s.z2 = 0.0;
        }
    }

    pub fn process(&mut self, x: f64) -> f64 {
        self.sections
            .iter_mut()
            .fold(x * self.gain, |v, s| s.process(v))
    }

    pub fn process_block(&mut self, samples: &mut [f32]) {
        for s in samples {
            *s = self.process(*s as f64) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analytic_curves() {
        assert!(Weighting::A.db_at(1000.0).abs() < 0.01);
        assert!((Weighting::A.db_at(100.0) + 19.1).abs() < 0.1);
        assert!((Weighting::A.db_at(10_000.0) + 2.5).abs() < 0.1);
        assert!(Weighting::C.db_at(1000.0).abs() < 0.01);
        assert!((Weighting::C.db_at(31.5) + 3.0).abs() < 0.1);
    }

    #[test]
    fn digital_filter_matches_analytic_in_band() {
        let fs = 48_000;
        let f = WeightingFilter::new(Weighting::A, fs);
        // IEC 61672-1 class 1 tolerances (+upper, −lower, dB) at the checked frequencies;
        // the digital curve falls below the analogue one near Nyquist, which the standard
        // tolerates far more than an excess.
        for (hz, up, down) in [
            (31.5, 1.0, 1.0),
            (100.0, 0.2, 0.2),
            (1000.0, 0.01, 0.01),
            (4000.0, 0.7, 0.7),
            (10_000.0, 1.5, 2.0),
            (16_000.0, 2.5, 16.0),
        ] {
            let d = f.db_at(hz, fs);
            let a = Weighting::A.db_at(hz);
            assert!(
                d - a < up && a - d < down,
                "{hz} Hz: digital {d} vs analytic {a}"
            );
        }
        // Time domain: a 100 Hz sine comes out at -19.1 dB.
        let mut f = WeightingFilter::new(Weighting::A, fs);
        let mut x: Vec<f32> = (0..96_000)
            .map(|i| (std::f64::consts::TAU * 100.0 * i as f64 / fs as f64).sin() as f32)
            .collect();
        f.process_block(&mut x);
        let tail = &x[48_000..];
        let rms =
            (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt();
        let level = db(rms * std::f64::consts::SQRT_2);
        assert!((level + 19.1).abs() < 0.2, "{level}");
    }
}
