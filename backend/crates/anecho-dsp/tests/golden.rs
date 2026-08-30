//! Golden tests: deterministic synthetic fixtures → snapshotted results.
//!
//! Any change to a snapshot is a change of the backend's numerical behaviour and must be
//! called out explicitly (CLAUDE.md rule 5). Review with `cargo insta review`.

use anecho_dsp::spectrum::band_power;
use anecho_dsp::{
    Averager, Averaging, Imd, Level, LogAxis, OctaveBands, RealSpectrum, Signal, SignalGen, Thd,
    ThdOptions, Window, db_peak,
};
use serde::Serialize;

const FS: u32 = 48_000;
const N: usize = 32_768;

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn render(signal: Signal, level_dbfs: f64, n: usize) -> Vec<f32> {
    let mut g = SignalGen::new(signal, FS, Level::Dbfs(level_dbfs));
    let mut x = vec![0f32; n];
    g.fill(&mut x);
    x
}

fn add_harmonics(x: &mut [f32], f0: f64, peak: f64, harmonics: &[(u32, f64)]) {
    for (i, s) in x.iter_mut().enumerate() {
        let t = i as f64 / FS as f64;
        let mut v = 0.0;
        for (n, db) in harmonics {
            v += peak * 10f64.powf(db / 20.0) * (std::f64::consts::TAU * *n as f64 * f0 * t).sin();
        }
        *s += v as f32;
    }
}

#[derive(Serialize)]
struct ThdSnapshot {
    fundamental_hz: f64,
    fundamental_level_db: f64,
    thd_pct: f64,
    thd_db: f64,
    thd_n_pct: f64,
    harmonics_db_rel: Vec<(u32, f64)>,
    noise_floor_db: f64,
}

fn thd_snapshot(x: &[f32], window: Window) -> ThdSnapshot {
    let r = Thd::analyze(
        x,
        FS as f64,
        &ThdOptions {
            window,
            ..Default::default()
        },
    );
    ThdSnapshot {
        fundamental_hz: r3(r.fundamental_hz),
        fundamental_level_db: r3(r.fundamental_level_db),
        thd_pct: (r.thd_pct * 1e5).round() / 1e5,
        thd_db: r3(r.thd_db),
        thd_n_pct: (r.thd_n_pct * 1e5).round() / 1e5,
        harmonics_db_rel: r
            .harmonics
            .iter()
            .map(|h| (h.n, r3(h.level_db_rel)))
            .collect(),
        noise_floor_db: r3(r.noise_floor_db),
    }
}

#[test]
fn thd_sine_with_h2_h3() {
    let mut x = render(Signal::Sine { hz: 1000.0 }, -6.0206, N);
    add_harmonics(&mut x, 1000.0, 0.5, &[(2, -60.0), (3, -70.0)]);
    insta::assert_json_snapshot!(
        "thd_sine_h2_h3_bh7",
        thd_snapshot(&x, Window::BlackmanHarris7)
    );
    insta::assert_json_snapshot!("thd_sine_h2_h3_hann", thd_snapshot(&x, Window::Hann));
}

#[test]
fn thd_sine_in_noise() {
    let mut x = render(Signal::Sine { hz: 997.0 }, -10.0, N);
    let noise = render(Signal::WhiteNoise { seed: 42 }, -80.0, N);
    for (s, n) in x.iter_mut().zip(noise) {
        *s += n;
    }
    insta::assert_json_snapshot!(
        "thd_sine_997_in_white_noise",
        thd_snapshot(&x, Window::BlackmanHarris7)
    );
}

#[test]
fn imd_smpte_and_ccif() {
    // Clean dual tones: the residual is the analysis floor of the window itself.
    let smpte = render(
        Signal::DualTone {
            f1: 60.0,
            f2: 7000.0,
            ratio_db: 12.0412,
        },
        -6.0,
        N,
    );
    let r = Imd::smpte(&smpte, FS as f64, Window::BlackmanHarris7, 60.0, 7000.0);
    insta::assert_json_snapshot!(
        "imd_smpte_clean",
        (
            (r.imd_pct * 1e6).round() / 1e6,
            r.products
                .iter()
                .map(|(hz, db)| (r3(*hz), r3(*db)))
                .collect::<Vec<_>>()
        )
    );
    // Modulated 7 kHz tone: 1 % AM → sidebands −46.02 dB each → IMD 0.707 %.
    let mut modulated = render(Signal::Sine { hz: 60.0 }, -8.0, N);
    for (i, s) in modulated.iter_mut().enumerate() {
        let t = i as f64 / FS as f64;
        *s += (0.1
            * (1.0 + 0.01 * (std::f64::consts::TAU * 60.0 * t).cos())
            * (std::f64::consts::TAU * 7000.0 * t).sin()) as f32;
    }
    let r = Imd::smpte(&modulated, FS as f64, Window::BlackmanHarris7, 60.0, 7000.0);
    insta::assert_json_snapshot!(
        "imd_smpte_1pct_am",
        ((r.imd_pct * 1e4).round() / 1e4, r3(r.imd_db))
    );

    let ccif = render(
        Signal::DualTone {
            f1: 19_000.0,
            f2: 20_000.0,
            ratio_db: 0.0,
        },
        -6.0,
        N,
    );
    let r = Imd::ccif(
        &ccif,
        FS as f64,
        Window::BlackmanHarris7,
        19_000.0,
        20_000.0,
    );
    insta::assert_json_snapshot!(
        "imd_ccif_clean",
        (
            (r.imd_pct * 1e6).round() / 1e6,
            r.products
                .iter()
                .map(|(hz, db)| (r3(*hz), r3(*db)))
                .collect::<Vec<_>>()
        )
    );
}

#[test]
fn rta_pipeline_on_pink_noise() {
    // Pink noise → averaged power → octave bands and a log display axis.
    let mut g = SignalGen::new(Signal::PinkNoise { seed: 1 }, FS, Level::Dbfs(-10.0));
    let mut fft = RealSpectrum::new(8192);
    let mut avg = Averager::new(Averaging::Linear { n: 32 });
    let mut x = vec![0f32; 8192];
    for _ in 0..32 {
        g.fill(&mut x);
        avg.push(&fft.power(&x, Window::Hann));
    }
    let power = avg.push(&vec![0.0; 4097]).to_vec(); // frozen: extra push ignored
    let bands = OctaveBands::new(1, 31.5, 16_000.0);
    let band_db: Vec<(f64, f64)> = bands
        .centres()
        .iter()
        .zip(bands.band_powers(&power, FS as f64, Window::Hann))
        .map(|(c, p)| (r3(*c), r3(10.0 * p.log10())))
        .collect();
    let axis = LogAxis::new(20.0, 20_000.0, 24);
    let display: Vec<f64> = axis
        .decimate(&power, FS as f64)
        .iter()
        .map(|p| r3(db_peak(p.sqrt())))
        .collect();
    let total = band_power(&power, FS as f64, Window::Hann, 20.0, 20_000.0);
    insta::assert_json_snapshot!("rta_pink_noise_octaves", band_db);
    insta::assert_json_snapshot!("rta_pink_noise_log_axis_24pts", display);
    insta::assert_json_snapshot!("rta_pink_noise_total_power_db", r3(10.0 * total.log10()));
}

#[test]
fn generator_levels() {
    #[derive(Serialize)]
    struct Gen {
        name: &'static str,
        peak: f64,
        rms_db: f64,
    }
    let mut out = Vec::new();
    for (name, sig) in [
        ("sine", Signal::Sine { hz: 1000.0 }),
        ("square", Signal::Square { hz: 1000.0 }),
        (
            "dual_smpte",
            Signal::DualTone {
                f1: 60.0,
                f2: 7000.0,
                ratio_db: 12.0412,
            },
        ),
        (
            "multitone_10",
            Signal::Multitone {
                tones: (1..=10).map(|k| (100.0 * k as f64, 0.0)).collect(),
                schroeder: true,
            },
        ),
        ("white", Signal::WhiteNoise { seed: 5 }),
        ("pink", Signal::PinkNoise { seed: 5 }),
        (
            "periodic_4096",
            Signal::PeriodicNoise {
                seed: 5,
                period_frames: 4096,
            },
        ),
    ] {
        let x = render(sig, -12.0, N);
        let peak = x.iter().fold(0f32, |m, v| m.max(v.abs())) as f64;
        let rms = (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
        out.push(Gen {
            name,
            peak: r3(peak),
            rms_db: r3(20.0 * rms.log10()),
        });
    }
    insta::assert_json_snapshot!("generator_levels_minus_12_dbfs", out);
}
