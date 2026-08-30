//! Test signal generators (mono, phase-continuous, deterministic).
//!
//! Level convention: [`Level::Dbfs`] is a **peak** level (full-scale sine = 0 dBFS).
//! - Sine / square: peak = level.
//! - Dual tone: `a_1 + a_2 = peak`, `a_1 / a_2 = 10^(ratio_db/20)` (SMPTE 4:1 = 12.04 dB).
//! - Multitone: the peak of the *sum* equals the level (measured over one second of signal
//!   at construction, deterministic).
//! - Noise: RMS = peak/√2 (the RMS of a sine with that peak), samples clamped to ±1.
//!
//! Pink noise uses Paul Kellet's refined 7-state filter (−3 dB/octave within ±0.05 dB over
//! 9.2 Hz–fs/2 at 44.1 kHz):
//! `b0 = 0.99886 b0 + w·0.0555179; b1 = 0.99332 b1 + w·0.0750759; b2 = 0.96900 b2 + w·0.1538520;
//!  b3 = 0.86650 b3 + w·0.3104856; b4 = 0.55000 b4 + w·0.5329522; b5 = −0.7616 b5 − w·0.0168980;
//!  pink = b0+…+b6 + w·0.5362; b6 = w·0.115926`.
//! White noise is uniform in [−1, 1) from a xoshiro256** generator seeded by the caller.
//! Periodic noise loops one buffer of `period_frames` white samples, so the DFT of one period
//! is exactly line-spectral (no leakage when analysed with a matching FFT length).
//! Multitone phases use Schroeder's rule `φ_k = π k² / K` when `schroeder` is set (low crest).

use crate::units::from_db;
use std::f64::consts::{PI, TAU};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Signal {
    Sine {
        hz: f64,
    },
    DualTone {
        f1: f64,
        f2: f64,
        ratio_db: f64,
    },
    Multitone {
        tones: Vec<(f64, f64)>,
        schroeder: bool,
    },
    WhiteNoise {
        seed: u64,
    },
    PinkNoise {
        seed: u64,
    },
    PeriodicNoise {
        seed: u64,
        period_frames: usize,
    },
    Square {
        hz: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Level {
    /// Peak level in dBFS.
    Dbfs(f64),
    /// Peak amplitude, linear (1.0 = full scale).
    Linear(f64),
}

impl Level {
    pub fn peak(self) -> f64 {
        match self {
            Level::Dbfs(db) => from_db(db),
            Level::Linear(p) => p,
        }
    }
}

/// xoshiro256** (Blackman & Vigna), seeded through splitmix64.
#[derive(Debug, Clone)]
struct Rng([u64; 4]);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut s = seed;
        let mut next = || {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Self([next(), next(), next(), next()])
    }

    fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in [-1, 1).
    fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

#[derive(Debug, Clone)]
enum State {
    Sine {
        step: f64,
        phase: f64,
    },
    DualTone {
        step1: f64,
        step2: f64,
        p1: f64,
        p2: f64,
        a1: f64,
        a2: f64,
    },
    Multitone {
        steps: Vec<f64>,
        phases: Vec<f64>,
        amp: f64,
    },
    White {
        rng: Rng,
        scale: f64,
    },
    Pink {
        rng: Rng,
        b: [f64; 7],
        scale: f64,
    },
    Periodic {
        buf: Vec<f32>,
        pos: usize,
    },
    Square {
        step: f64,
        phase: f64,
    },
}

/// Streaming generator.
#[derive(Debug, Clone)]
pub struct SignalGen {
    peak: f64,
    state: State,
}

/// RMS of uniform noise in [-1, 1): 1/√3.
const UNIFORM_RMS: f64 = 0.577_350_269_189_625_8;

impl SignalGen {
    pub fn new(signal: Signal, sample_rate: u32, level: Level) -> Self {
        let fs = sample_rate as f64;
        let peak = level.peak();
        let target_rms = peak / std::f64::consts::SQRT_2;
        let state = match signal {
            Signal::Sine { hz } => State::Sine {
                step: TAU * hz / fs,
                phase: 0.0,
            },
            Signal::DualTone { f1, f2, ratio_db } => {
                let r = from_db(ratio_db);
                let a2 = 1.0 / (1.0 + r);
                State::DualTone {
                    step1: TAU * f1 / fs,
                    step2: TAU * f2 / fs,
                    p1: 0.0,
                    p2: 0.0,
                    a1: r * a2,
                    a2,
                }
            }
            Signal::Multitone { tones, schroeder } => {
                let k = tones.len().max(1) as f64;
                let phases: Vec<f64> = tones
                    .iter()
                    .enumerate()
                    .map(|(i, (_, p))| {
                        if schroeder {
                            PI * (i * i) as f64 / k
                        } else {
                            *p
                        }
                    })
                    .collect();
                let steps: Vec<f64> = tones.iter().map(|(hz, _)| TAU * hz / fs).collect();
                // Peak of the sum over one second, for normalisation.
                let mut max = 0f64;
                for n in 0..sample_rate as usize {
                    let v: f64 = steps
                        .iter()
                        .zip(&phases)
                        .map(|(s, p)| (s * n as f64 + p).sin())
                        .sum();
                    max = max.max(v.abs());
                }
                State::Multitone {
                    steps,
                    phases,
                    amp: if max > 0.0 { 1.0 / max } else { 0.0 },
                }
            }
            Signal::WhiteNoise { seed } => State::White {
                rng: Rng::new(seed),
                scale: target_rms / UNIFORM_RMS / peak.max(1e-12),
            },
            Signal::PinkNoise { seed } => {
                // Measure the filter's output RMS for unit-RMS white input, deterministically.
                let mut probe = Rng::new(seed ^ 0xA5A5_A5A5);
                let mut b = [0f64; 7];
                let mut sum_sq = 0.0;
                let n = 65_536;
                for _ in 0..n {
                    let w = probe.uniform() / UNIFORM_RMS;
                    sum_sq += pink_step(&mut b, w).powi(2);
                }
                let rms = (sum_sq / n as f64).sqrt();
                State::Pink {
                    rng: Rng::new(seed),
                    b: [0.0; 7],
                    scale: target_rms / (UNIFORM_RMS * rms) / peak.max(1e-12),
                }
            }
            Signal::PeriodicNoise {
                seed,
                period_frames,
            } => {
                let mut rng = Rng::new(seed);
                let scale = target_rms / UNIFORM_RMS / peak.max(1e-12);
                let buf = (0..period_frames.max(1))
                    .map(|_| (peak * scale * rng.uniform()).clamp(-1.0, 1.0) as f32)
                    .collect();
                State::Periodic { buf, pos: 0 }
            }
            Signal::Square { hz } => State::Square {
                step: TAU * hz / fs,
                phase: 0.0,
            },
        };
        Self { peak, state }
    }

    /// Requested peak amplitude (linear).
    pub fn peak(&self) -> f64 {
        self.peak
    }

    /// Fill a mono buffer, continuing from the previous call.
    pub fn fill(&mut self, out: &mut [f32]) {
        let peak = self.peak;
        match &mut self.state {
            State::Sine { step, phase } => {
                for o in out {
                    *o = (peak * phase.sin()) as f32;
                    *phase = (*phase + *step) % TAU;
                }
            }
            State::DualTone {
                step1,
                step2,
                p1,
                p2,
                a1,
                a2,
            } => {
                for o in out {
                    *o = (peak * (*a1 * p1.sin() + *a2 * p2.sin())) as f32;
                    *p1 = (*p1 + *step1) % TAU;
                    *p2 = (*p2 + *step2) % TAU;
                }
            }
            State::Multitone { steps, phases, amp } => {
                for o in out {
                    let v: f64 = phases.iter().map(|p| p.sin()).sum();
                    *o = (peak * *amp * v) as f32;
                    for (p, s) in phases.iter_mut().zip(steps.iter()) {
                        *p = (*p + *s) % TAU;
                    }
                }
            }
            State::White { rng, scale } => {
                for o in out {
                    *o = (peak * *scale * rng.uniform()).clamp(-1.0, 1.0) as f32;
                }
            }
            State::Pink { rng, b, scale } => {
                for o in out {
                    let w = rng.uniform();
                    *o = (peak * *scale * pink_step(b, w)).clamp(-1.0, 1.0) as f32;
                }
            }
            State::Periodic { buf, pos } => {
                for o in out {
                    *o = buf[*pos];
                    *pos = (*pos + 1) % buf.len();
                }
            }
            State::Square { step, phase } => {
                for o in out {
                    *o = (if phase.sin() >= 0.0 { peak } else { -peak }) as f32;
                    *phase = (*phase + *step) % TAU;
                }
            }
        }
    }
}

fn pink_step(b: &mut [f64; 7], w: f64) -> f64 {
    b[0] = 0.99886 * b[0] + w * 0.055_517_9;
    b[1] = 0.99332 * b[1] + w * 0.075_075_9;
    b[2] = 0.96900 * b[2] + w * 0.153_852_0;
    b[3] = 0.86650 * b[3] + w * 0.310_485_6;
    b[4] = 0.55000 * b[4] + w * 0.532_952_2;
    b[5] = -0.7616 * b[5] - w * 0.016_898_0;
    let pink = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
    b[6] = w * 0.115_926;
    pink
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::psd;
    use crate::window::Window;

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn sine_level_and_continuity() {
        let mut g = SignalGen::new(Signal::Sine { hz: 1000.0 }, 48_000, Level::Dbfs(-6.0206));
        let mut a = vec![0f32; 480];
        let mut b = vec![0f32; 480];
        g.fill(&mut a);
        g.fill(&mut b);
        let peak = a.iter().chain(&b).fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 1e-4);
        assert!((rms(&a) - 0.5 / 2f64.sqrt()).abs() < 1e-3);
        // 1 kHz at 48 kHz has a 48-sample period: b continues a exactly.
        assert!((a[0] - b[0]).abs() < 1e-6);
    }

    #[test]
    fn noise_levels() {
        for sig in [
            Signal::WhiteNoise { seed: 1 },
            Signal::PinkNoise { seed: 1 },
        ] {
            let mut g = SignalGen::new(sig.clone(), 48_000, Level::Dbfs(-20.0));
            let mut x = vec![0f32; 1 << 16];
            g.fill(&mut x);
            let expected = 0.1 / 2f64.sqrt();
            let err = 20.0 * (rms(&x) / expected).log10();
            assert!(err.abs() < 0.3, "{sig:?}: {err} dB");
        }
    }

    #[test]
    fn pink_noise_slope_is_minus_3_db_per_octave() {
        let fs = 48_000.0;
        let mut g = SignalGen::new(Signal::PinkNoise { seed: 7 }, 48_000, Level::Dbfs(-10.0));
        let n = 1 << 16;
        let mut acc = vec![0.0; n / 2 + 1];
        for _ in 0..16 {
            let mut x = vec![0f32; n];
            g.fill(&mut x);
            for (a, p) in acc.iter_mut().zip(psd(&x, Window::Hann, fs)) {
                *a += p / 16.0;
            }
        }
        let bin = |hz: f64| (hz * n as f64 / fs) as usize;
        let band_db = |lo: f64, hi: f64| {
            let (a, b) = (bin(lo), bin(hi));
            10.0 * (acc[a..b].iter().sum::<f64>() / (b - a) as f64).log10()
        };
        // Octave-band PSD means: 100–200 Hz vs 5–10 kHz = 5.64 octaves apart → −16.9 dB.
        let slope = (band_db(5000.0, 10_000.0) - band_db(100.0, 200.0)) / (50.0f64).log2();
        assert!((slope + 3.0).abs() < 0.5, "slope {slope} dB/oct");
    }

    #[test]
    fn periodic_noise_repeats_exactly() {
        let mut g = SignalGen::new(
            Signal::PeriodicNoise {
                seed: 3,
                period_frames: 1024,
            },
            48_000,
            Level::Dbfs(-6.0),
        );
        let mut x = vec![0f32; 3072];
        g.fill(&mut x);
        assert_eq!(&x[..1024], &x[1024..2048]);
        assert_eq!(&x[..1024], &x[2048..]);
    }

    #[test]
    fn dual_tone_ratio_and_peak() {
        let mut g = SignalGen::new(
            Signal::DualTone {
                f1: 60.0,
                f2: 7000.0,
                ratio_db: 12.0412,
            },
            48_000,
            Level::Linear(1.0),
        );
        let mut x = vec![0f32; 48_000];
        g.fill(&mut x);
        let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak <= 1.0 + 1e-6 && peak > 0.98, "{peak}");
    }

    #[test]
    fn multitone_peak_is_normalised() {
        let tones: Vec<(f64, f64)> = (1..=10).map(|k| (100.0 * k as f64, 0.0)).collect();
        let mut g = SignalGen::new(
            Signal::Multitone {
                tones,
                schroeder: true,
            },
            48_000,
            Level::Dbfs(-3.0),
        );
        let mut x = vec![0f32; 48_000];
        g.fill(&mut x);
        let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak as f64 - from_db(-3.0)).abs() < 1e-3, "{peak}");
    }
}
