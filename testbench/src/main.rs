use anecho_testbench::{Check, anecho, rew};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "anecho-testbench", about = "A/B bench: Anecho vs REW")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Drive REW alone through a loopback device: generator sine -> RTA -> peak and THD.
    /// Validates the REW client and the loopback wiring before any A/B comparison.
    RewRta {
        #[arg(long, default_value = rew::DEFAULT_BASE_URL)]
        rew: String,
        /// REW device name used for both input and output (a loopback device).
        #[arg(long, default_value = "BlackHole 2ch")]
        device: String,
        #[arg(long, default_value_t = 1000.0)]
        frequency_hz: f64,
        #[arg(long, default_value_t = -12.0)]
        level_dbfs: f64,
        #[arg(long, default_value = "Hann")]
        window: String,
        #[arg(long, default_value = "64k")]
        fft_length: String,
        #[arg(long, default_value_t = 4.0)]
        seconds: f64,
    },
    /// Compare versions and device lists of both engines.
    Compare {
        #[arg(long, default_value = rew::DEFAULT_BASE_URL)]
        rew: String,
        #[arg(long, default_value = anecho::DEFAULT_URL)]
        anecho: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::RewRta {
            rew: rew_url,
            device,
            frequency_hz,
            level_dbfs,
            window,
            fft_length,
            seconds,
        } => {
            let rew = rew::Rew::new(&rew_url);
            let (app, api) = rew.version().await?.split();
            println!("REW {app} (API {api})");
            let prev_in = rew.input_device_name().await?;
            let prev_out = rew.output_device_name().await?;
            let prev_cfg = rew.rta_configuration().await?;
            rew.set_input_device(&device).await?;
            rew.set_output_device(&device).await?;
            rew.set_sample_rate(48_000.0).await?;
            let cfg = rew::RtaConfiguration {
                mode: "Spectrum".into(),
                smoothing: "None".into(),
                fft_length: fft_length.clone(),
                window: window.clone(),
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
            rew.rta_command("Start").await?;
            tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
            let spectrum = rew.rta_captured_data("dBFS").await;
            let distortion = rew.rta_distortion().await;
            rew.rta_command("Stop").await?;
            rew.generator_command("Stop").await?;
            rew.set_rta_configuration(&prev_cfg).await?;
            rew.set_input_device(&prev_in).await?;
            rew.set_output_device(&prev_out).await?;

            let spectrum = spectrum?;
            let distortion_ok = distortion
                .as_ref()
                .map(|d| d.clone())
                .map_err(|e| e.to_string());
            let (peak_hz, peak_db) = spectrum.peak().unwrap_or((0.0, f32::NAN));
            let at_f = spectrum.magnitude[spectrum.bin_at(frequency_hz)];
            println!(
                "RTA {} {} on {device}: {} bins, {:.3} Hz/bin, peak {:.1} Hz = {:.2} {}, bin at {:.0} Hz = {:.2} {}",
                cfg.window,
                cfg.fft_length,
                spectrum.magnitude.len(),
                spectrum.step_hz,
                peak_hz,
                peak_db,
                spectrum.unit,
                frequency_hz,
                at_f,
                spectrum.unit
            );
            match distortion {
                Ok(d) => println!(
                    "distortion: f0 {:.2} Hz at {:.2} dBFS, THD {} {}, THD+N {} {}, {} averages",
                    d.fundamental_frequency,
                    d.fundamental_dbfs,
                    d.thd
                        .as_ref()
                        .map(|v| format!("{:.4}", v.value))
                        .unwrap_or_default(),
                    d.thd.as_ref().map(|v| v.unit.as_str()).unwrap_or(""),
                    d.thd_plus_n
                        .as_ref()
                        .map(|v| format!("{:.4}", v.value))
                        .unwrap_or_default(),
                    d.thd_plus_n.as_ref().map(|v| v.unit.as_str()).unwrap_or(""),
                    d.averages
                ),
                Err(e) => println!("distortion: {e}"),
            }
            // The raw bin carries the window's scalloping loss when the tone is not
            // bin-centred (Hann: up to 1.4 dB); REW's distortion block interpolates the
            // fundamental, so that is the number to check the loopback level against.
            println!(
                "scalloping at {frequency_hz:.0} Hz: {:+.2} dB",
                at_f as f64 - level_dbfs
            );
            if let Ok(d) = &distortion_ok {
                let err = d.fundamental_dbfs - level_dbfs;
                println!("fundamental error vs generator: {err:+.2} dB");
                if err.abs() > 0.1 {
                    anyhow::bail!("REW loopback level off by {err:+.2} dB");
                }
            }
        }
        Cmd::Compare {
            rew: rew_url,
            anecho: anecho_url,
        } => {
            let rew = rew::Rew::new(&rew_url);
            let rv = rew.version().await?;
            let (rew_app, rew_api) = rv.split();
            let rew_in = rew.input_devices().await?;
            let rew_out = rew.output_devices().await?;
            let rew_meas = rew.measurements().await.map(|m| m.len()).unwrap_or(0);

            let a = anecho::summary(&anecho_url).await?;
            let a_in = a.devices.iter().filter(|d| d.input_channels > 0).count();
            let a_out = a.devices.iter().filter(|d| d.output_channels > 0).count();
            let a_cal = a.devices.iter().filter(|d| d.factory_calibrated).count();

            let checks = vec![
                Check {
                    name: "version".into(),
                    rew: format!("{rew_app} (API {rew_api})"),
                    anecho: format!("{} (contract {})", a.backend_version, a.contract_version),
                    ok: true,
                },
                Check {
                    name: "input devices".into(),
                    rew: rew_in.len().to_string(),
                    anecho: a_in.to_string(),
                    ok: !rew_in.is_empty() && a_in > 0,
                },
                Check {
                    name: "output devices".into(),
                    rew: rew_out.len().to_string(),
                    anecho: a_out.to_string(),
                    ok: !rew_out.is_empty() && a_out > 0,
                },
                Check {
                    name: "factory-calibrated".into(),
                    rew: "0 (REW has no QA40x support)".into(),
                    anecho: a_cal.to_string(),
                    ok: true,
                },
                Check {
                    name: "loaded measurements".into(),
                    rew: rew_meas.to_string(),
                    anecho: "n/a (phase 2)".into(),
                    ok: true,
                },
            ];
            let mut failed = false;
            for c in &checks {
                println!("{c}");
                failed |= !c.ok;
            }
            println!("\nREW input devices:");
            for d in &rew_in {
                println!("  - {d}");
            }
            println!("Anecho devices:");
            for d in &a.devices {
                println!(
                    "  - {} [{}]{}",
                    d.display_name,
                    d.id,
                    if d.factory_calibrated {
                        " calibrated"
                    } else {
                        ""
                    }
                );
            }
            if failed {
                anyhow::bail!("some checks failed");
            }
        }
    }
    Ok(())
}
