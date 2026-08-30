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
