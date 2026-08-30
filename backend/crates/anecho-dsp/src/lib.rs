//! Signal processing for Anecho — pure computation.
//!
//! See the crate README for the conventions (dBFS = peak, RMS labelled explicitly,
//! base-2 octaves, main-lobe harmonic sums).

pub mod distortion;
pub mod fft;
pub mod generator;
pub mod spectrum;
pub mod units;
pub mod weighting;
pub mod window;

pub use distortion::{DistortionResult, Harmonic, Imd, ImdResult, Thd, ThdOptions};
pub use fft::{RealSpectrum, db_peak, db_rms, magnitude_spectrum, psd};
pub use generator::{Level, Signal, SignalGen};
pub use spectrum::{Averager, Averaging, LogAxis, OctaveBands};
pub use window::Window;
