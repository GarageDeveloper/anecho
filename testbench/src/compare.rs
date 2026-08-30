//! Numerical A/B comparisons (phase 1: stationary sine through a loopback device).
//!
//! Conventions: REW reports sine levels with the **peak** convention (full-scale sine =
//! 0 dBFS) both in `/rta/captured-data` and in `/rta/distortion`; Anecho's LEVELS/RTA
//! values and `fundamental_level` are **RMS** (a full-scale sine reads -3.01 dBFS_rms).
//! Everything is compared in the peak convention: Anecho RMS values get +3.01 dB.

use crate::Check;
use crate::rew::{self, Rew};
use anecho_client::Client;
use anecho_contract::v0 as pb;
use std::time::Duration;

const RMS_TO_PEAK_DB: f64 = 3.010_299_956_639_812;

/// What one engine reports for a sine at `frequency_hz`.
#[derive(Debug, Clone)]
pub struct SineFigures {
    pub engine: String,
    pub window: String,
    pub fft_length: usize,
    /// Interpolated fundamental level, peak dBFS.
    pub fundamental_dbfs: f64,
    pub fundamental_hz: f64,
    /// Highest spectrum bin within ±5 Hz of the tone, peak dBFS (same window and FFT
    /// length on both sides, so the scalloping loss is identical).
    pub rta_peak_dbfs: f64,
    pub thd_db: f64,
    pub thd_n_db: f64,
}

pub async fn rew_thd(
    base_url: &str,
    device: &str,
    frequency_hz: f64,
    level_dbfs: f64,
    seconds: f64,
) -> anyhow::Result<SineFigures> {
    let rew = Rew::new(base_url);
    let prev_in = rew.input_device_name().await?;
    let prev_out = rew.output_device_name().await?;
    let prev_cfg = rew.rta_configuration().await?;
    rew.set_input_device(device).await?;
    rew.set_output_device(device).await?;
    rew.set_sample_rate(48_000.0).await?;
    let cfg = rew::RtaConfiguration {
        mode: "Spectrum".into(),
        smoothing: "None".into(),
        fft_length: "64k".into(),
        window: "Blackman-Harris 7".into(),
        averaging: "4".into(),
        calc_distortion_enabled: true,
        fundamental_from_sine_gen: true,
        ..prev_cfg.clone()
    };
    rew.set_rta_configuration(&cfg).await?;
    rew.set_generator_signal("sine").await?;
    rew.set_sine_frequency(frequency_hz).await?;
    rew.set_generator_level(level_dbfs, "dBFS").await?;
    rew.generator_command("Play").await?;
    // Let the generator settle before capturing, then wait for the linear average to
    // complete: 4 × 65536 frames at 48 kHz is 5.5 s of audio, plus REW's own overlap.
    tokio::time::sleep(Duration::from_millis(500)).await;
    rew.rta_command("Start").await?;
    let averages = 4.0;
    let needed = 1.5 * averages * 65_536.0 / 48_000.0 + 1.0;
    tokio::time::sleep(Duration::from_secs_f64(seconds.max(needed))).await;
    let spectrum = rew.rta_captured_data("dBFS").await;
    let distortion = rew.rta_distortion().await;
    rew.rta_command("Stop").await?;
    rew.generator_command("Stop").await?;
    rew.set_rta_configuration(&prev_cfg).await?;
    rew.set_input_device(&prev_in).await?;
    rew.set_output_device(&prev_out).await?;
    let spectrum = spectrum?;
    let d = distortion?;
    let lo = spectrum.bin_at(frequency_hz - 5.0);
    let hi = spectrum.bin_at(frequency_hz + 5.0);
    let rta_peak = spectrum.magnitude[lo..=hi]
        .iter()
        .copied()
        .fold(f32::MIN, f32::max) as f64;
    Ok(SineFigures {
        engine: "REW".into(),
        window: cfg.window,
        fft_length: 65_536,
        fundamental_dbfs: d.fundamental_dbfs,
        fundamental_hz: d.fundamental_frequency,
        rta_peak_dbfs: rta_peak,
        thd_db: d.thd.map(|v| v.value).unwrap_or(f64::NAN),
        thd_n_db: d.thd_plus_n.map(|v| v.value).unwrap_or(f64::NAN),
    })
}

pub async fn anecho_thd(
    url: &str,
    device_substring: &str,
    frequency_hz: f64,
    level_dbfs: f64,
) -> anyhow::Result<SineFigures> {
    let client = Client::connect(url).await?;
    let devices = client.list_devices().await?;
    let dev = devices
        .iter()
        .find(|d| d.id.contains(device_substring) && d.input_channels > 0 && d.output_channels > 0)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no Anecho device matching {device_substring:?} with inputs and outputs"
            )
        })?;
    let session = client
        .open_session(
            &dev.id,
            pb::DeviceConfig {
                sample_rate: 48_000,
                ..Default::default()
            },
        )
        .await?;
    let generator = pb::Generator {
        signal: Some(pb::generator::Signal::Sine(pb::generator::Sine {
            frequency_hz: frequency_hz as f32,
            amplitude_dbfs: level_dbfs as f32,
        })),
        ..Default::default()
    };
    // RTA: same FFT length and window as REW; a fine log axis so the cell holding the
    // tone is at most a few bins wide (max over the cell = the peak bin).
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            kind: pb::StreamKind::Rta as i32,
            generator: Some(generator.clone()),
            rta: Some(pb::RtaConfig {
                fft_length: 65_536,
                window: pb::rta_config::Window::BlackmanHarris7 as i32,
                averaging: Some(pb::rta_config::Averaging {
                    mode: pb::rta_config::averaging::Mode::Linear as i32,
                    count: 4,
                }),
                min_hz: 20.0,
                max_hz: 20_000.0,
                points: 4000,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await?;
    let mut last = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut n = 0;
    while n < 5 {
        let f = tokio::time::timeout_at(deadline, frames.recv()).await??;
        if f.stream_id == stream.stream_id {
            n += 1;
            last = Some(f);
        }
    }
    client.stop_stream(stream.stream_id).await?;
    let f = last.unwrap();
    let axis = &stream.axis_hz;
    let rta_peak = axis
        .iter()
        .zip(f.channel(0))
        .filter(|(hz, _)| (**hz as f64 - frequency_hz).abs() <= 5.0)
        .map(|(_, v)| *v as f64)
        .fold(f64::MIN, f64::max)
        + RMS_TO_PEAK_DB;

    let m = client
        .measure(pb::MeasureRequest {
            session_id: session.session_id,
            kind: pb::MeasureKind::Thd as i32,
            generator: Some(generator),
            fft_length: 65_536,
            window: pb::rta_config::Window::BlackmanHarris7 as i32,
            averages: 4,
            ..Default::default()
        })
        .await?;
    client.close_session(session.session_id).await?;
    let r = m
        .per_channel
        .first()
        .ok_or_else(|| anyhow::anyhow!("no channel in measurement"))?;
    Ok(SineFigures {
        engine: "Anecho".into(),
        window: "Blackman-Harris 7".into(),
        fft_length: 65_536,
        fundamental_dbfs: r.fundamental_level as f64 + RMS_TO_PEAK_DB,
        fundamental_hz: r.fundamental_hz as f64,
        rta_peak_dbfs: rta_peak,
        thd_db: r.thd_db as f64,
        thd_n_db: r.thd_n_db as f64,
    })
}

/// Below this, a loopback is a bit-exact digital path (e.g. BlackHole): distortion is
/// then a property of each generator's own dither, not of a device, and REW and Anecho
/// legitimately disagree. Those lines become informational.
const DIGITAL_LOOPBACK_DB: f64 = -150.0;

pub fn thd_checks(r: &SineFigures, a: &SineFigures) -> Vec<Check> {
    let digital = a.thd_n_db < DIGITAL_LOOPBACK_DB || r.thd_n_db < DIGITAL_LOOPBACK_DB;
    let line = |name: &str, rv: f64, av: f64, tol: f64, info_if_digital: bool| Check {
        name: name.into(),
        rew: format!("{rv:9.2} dB"),
        anecho: if info_if_digital && digital {
            format!("{av:9.2} dB  (digital loopback: not comparable, informational)")
        } else {
            format!("{av:9.2} dB  (Δ {:+.2})", av - rv)
        },
        ok: (info_if_digital && digital) || (av - rv).abs() <= tol,
    };
    vec![
        Check {
            name: "setup".into(),
            rew: format!("{} {}", r.window, r.fft_length),
            anecho: format!("{} {}", a.window, a.fft_length),
            ok: true,
        },
        Check {
            name: "fundamental Hz".into(),
            rew: format!("{:.3}", r.fundamental_hz),
            anecho: format!("{:.3} (snapped to the bin grid)", a.fundamental_hz),
            ok: (a.fundamental_hz - r.fundamental_hz).abs() < 1.0,
        },
        line(
            "fundamental (peak dBFS)",
            r.fundamental_dbfs,
            a.fundamental_dbfs,
            0.1,
            false,
        ),
        line(
            "RTA peak bin (peak dBFS)",
            r.rta_peak_dbfs,
            a.rta_peak_dbfs,
            0.3,
            false,
        ),
        line("THD", r.thd_db, a.thd_db, 3.0, true),
        line("THD+N", r.thd_n_db, a.thd_n_db, 3.0, true),
    ]
}
