# anecho-dsp

Pure signal processing for Anecho: no I/O, no async, `f64` internally, `f32` at the audio
edges. Every public function documents its formula and its conventions.

## Conventions

- **dBFS** of a signal refers to its **peak**: a full-scale sine (peak 1.0) is 0 dBFS. RMS
  quantities are always labelled `_rms` (`units::dbfs_rms`, `fft::db_rms`) — the RMS of a
  full-scale sine is −3.01 dBFS_rms.
- Spectra (`fft::magnitude_spectrum`) are single-sided, per-bin **RMS amplitudes in linear
  full-scale units**, corrected by the window's coherent gain: a full-scale sine centred on a
  bin reads 1/√2. `fft::db_peak` turns that into dBFS (peak convention).
- Broadband power is `Σ power_bins / ENBW` (`spectrum::band_power`), which is exact for both
  sines (main lobe) and noise. PSD (`fft::psd`) is per Hz, ENBW-corrected.
- Octave bands are **base-2** (centre 1 kHz, `f_c = 1000·2^(k/fraction)`), like REW.
- Harmonic power in `distortion` sums the window's main lobe: `Window::main_lobe_bins()` bins
  on each side of the harmonic bin (rectangular 1, Hann 2, Blackman-Harris 4-term 4, 7-term
  5, flat-top 5), divided by ENBW.
- Noise generators are scaled so that their RMS equals the RMS of a sine with the requested
  peak level (`Level::Dbfs(peak)` → RMS = peak/√2); samples are clamped to ±1.

## Golden tests

`tests/golden.rs` builds deterministic fixtures in memory (sines with known harmonics, seeded
noise, SMPTE/CCIF pairs) and snapshots the results (`insta`, values rounded to 3 decimals).

```
cargo test -p anecho-dsp            # fails if a snapshot changed
cargo insta review                  # inspect and accept intended changes
```

Any snapshot change must be called out explicitly in the pull request (CLAUDE.md rule 5):
snapshots are the numerical behaviour of the backend, never regenerated silently.
