//! Analysis windows.
//!
//! All windows are *periodic* (DFT-even): `w[n] = Σ_k (−1)^k a_k cos(2π k n / N)`, which
//! makes a sine exactly on a bin leak nowhere. Coefficients:
//!
//! | window | a_0 … a_K |
//! |---|---|
//! | Rectangular | 1 |
//! | Hann | 0.5, 0.5 |
//! | Blackman-Harris 4-term | 0.35875, 0.48829, 0.14128, 0.01168 |
//! | Blackman-Harris 7-term | 0.27105140069342, 0.43329793923448, 0.21812299954311, 0.06592544638803, 0.01081174209837, 0.00077658482522, 0.00001388721735 |
//! | Flat-top (Matlab/ISO 18431-2) | 0.21557895, 0.41663158, 0.277263158, 0.083578947, 0.006947368 |
//!
//! Figures of merit: coherent gain `CG = Σw/N` (amplitude correction for a sine),
//! equivalent noise bandwidth `ENBW = N·Σw² / (Σw)²` in bins (power correction for noise),
//! scalloping loss (worst-case amplitude error for a sine half-way between bins:
//! rectangular 3.9 dB, Hann 1.4 dB, BH4 0.8 dB, flat-top < 0.01 dB).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type WindowCache = Mutex<HashMap<(Window, usize), Arc<[f64]>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Window {
    Rectangular,
    Hann,
    BlackmanHarris4,
    BlackmanHarris7,
    FlatTop,
}

impl Window {
    pub fn coefficients(self) -> &'static [f64] {
        match self {
            Window::Rectangular => &[1.0],
            Window::Hann => &[0.5, 0.5],
            Window::BlackmanHarris4 => &[0.35875, 0.48829, 0.14128, 0.01168],
            Window::BlackmanHarris7 => &[
                0.271_051_400_693_42,
                0.433_297_939_234_48,
                0.218_122_999_543_11,
                0.065_925_446_388_03,
                0.010_811_742_098_37,
                0.000_776_584_825_22,
                0.000_013_887_217_35,
            ],
            Window::FlatTop => &[
                0.215_578_95,
                0.416_631_58,
                0.277_263_158,
                0.083_578_947,
                0.006_947_368,
            ],
        }
    }

    /// Bins on each side of a sine's bin that hold its main lobe (used to sum the power of
    /// a spectral line): rectangular 1, Hann 2, Blackman-Harris 4-term 4, 7-term 7, flat-top 5.
    pub fn main_lobe_bins(self) -> usize {
        match self {
            Window::Rectangular => 1,
            Window::Hann => 2,
            Window::BlackmanHarris4 => 4,
            // Measured on a bin-centred tone: the 7-term window still carries -96 dBc at
            // ±6 bins and reaches its sidelobe floor (< -125 dBc) from ±8 on, so the lobe
            // is ±7 bins (2026-08-30, QA403 loopback, N = 65536).
            Window::BlackmanHarris7 => 7,
            Window::FlatTop => 5,
        }
    }

    /// Window samples, cached per (window, length).
    pub fn samples(self, len: usize) -> Arc<[f64]> {
        static CACHE: OnceLock<WindowCache> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Some(w) = cache.lock().unwrap().get(&(self, len)) {
            return w.clone();
        }
        let w: Arc<[f64]> = Arc::from(self.compute(len));
        cache.lock().unwrap().insert((self, len), w.clone());
        w
    }

    fn compute(self, len: usize) -> Vec<f64> {
        let a = self.coefficients();
        (0..len)
            .map(|n| {
                let x = std::f64::consts::TAU * n as f64 / len as f64;
                a.iter()
                    .enumerate()
                    .map(|(k, ak)| {
                        let sign = if k % 2 == 0 { *ak } else { -*ak };
                        sign * (k as f64 * x).cos()
                    })
                    .sum()
            })
            .collect()
    }

    /// Coherent gain `Σw / N` (independent of N for periodic windows: a_0).
    pub fn coherent_gain(self) -> f64 {
        self.coefficients()[0]
    }

    /// Equivalent noise bandwidth in bins: `N·Σw² / (Σw)²` = `(a_0² + Σ_{k>0} a_k²/2) / a_0²`.
    pub fn enbw_bins(self) -> f64 {
        let a = self.coefficients();
        let sum_sq: f64 = a
            .iter()
            .enumerate()
            .map(|(k, ak)| if k == 0 { ak * ak } else { ak * ak / 2.0 })
            .sum();
        sum_sq / (a[0] * a[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(w: Window, n: usize) -> (f64, f64) {
        let s = w.samples(n);
        let sum: f64 = s.iter().sum();
        let sum_sq: f64 = s.iter().map(|v| v * v).sum();
        (sum / n as f64, n as f64 * sum_sq / (sum * sum))
    }

    #[test]
    fn hann_figures() {
        let (cg, enbw) = measured(Window::Hann, 4096);
        assert!((cg - 0.5).abs() < 1e-9);
        assert!((enbw - 1.5).abs() < 1e-9);
        assert!((Window::Hann.coherent_gain() - 0.5).abs() < 1e-12);
        assert!((Window::Hann.enbw_bins() - 1.5).abs() < 1e-12);
    }

    #[test]
    fn closed_forms_match_measurements() {
        for w in [
            Window::Rectangular,
            Window::BlackmanHarris4,
            Window::BlackmanHarris7,
            Window::FlatTop,
        ] {
            let (cg, enbw) = measured(w, 8192);
            assert!((cg - w.coherent_gain()).abs() < 1e-9, "{w:?} cg");
            assert!((enbw - w.enbw_bins()).abs() < 1e-6, "{w:?} enbw");
        }
        assert!((Window::BlackmanHarris4.enbw_bins() - 2.0044).abs() < 1e-3);
        assert!((Window::FlatTop.enbw_bins() - 3.77).abs() < 0.01);
    }

    #[test]
    fn cache_returns_same_allocation() {
        let a = Window::Hann.samples(256);
        let b = Window::Hann.samples(256);
        assert!(Arc::ptr_eq(&a, &b));
    }
}
