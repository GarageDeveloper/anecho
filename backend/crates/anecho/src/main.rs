use anecho_client::Client;
use anecho_contract::v0 as pb;
use anyhow::Context;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "anecho",
    version,
    about = "Anecho audio analyzer — headless backend and CLI client"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the backend and serve the WebSocket API.
    Serve {
        #[arg(long, default_value_t = SocketAddr::from(([127, 0, 0, 1], anecho_server::DEFAULT_PORT)))]
        bind: SocketAddr,
        /// Also expose the virtual loopback device.
        #[arg(long)]
        virtual_loopback: bool,
        /// Do not expose sound cards.
        #[arg(long)]
        no_cpal: bool,
    },
    /// Print backend and contract versions.
    Version {
        #[arg(long, default_value = "ws://127.0.0.1:4800/ws")]
        url: String,
    },
    /// List devices known to a running backend.
    Devices {
        #[arg(long, default_value = "ws://127.0.0.1:4800/ws")]
        url: String,
    },
    /// Open a device and print live input levels.
    Levels {
        #[arg(long, default_value = "ws://127.0.0.1:4800/ws")]
        url: String,
        /// Device id as printed by `devices`.
        #[arg(long)]
        device: String,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Drive the outputs with a sine: `<frequency_hz>,<amplitude_dbfs>`.
        #[arg(long)]
        sine: Option<String>,
        #[arg(long, default_value_t = 2.0)]
        seconds: f64,
        #[arg(long, default_value_t = 10.0)]
        rate_hz: f32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match Cli::parse().cmd {
        Cmd::Serve {
            bind,
            virtual_loopback,
            no_cpal,
        } => {
            let engine = anecho::build_engine(&anecho::BackendOptions {
                virtual_loopback,
                cpal: !no_cpal,
            });
            let shutdown = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            let (addr, task) = anecho_server::serve(engine, bind, shutdown).await?;
            log::info!("anecho backend listening on ws://{addr}/ws");
            task.await??;
        }
        Cmd::Version { url } => {
            let v = Client::connect(&url).await?.version().await?;
            println!(
                "backend {} contract {}",
                v.backend_version, v.contract_version
            );
        }
        Cmd::Devices { url } => {
            for d in Client::connect(&url).await?.list_devices().await? {
                println!(
                    "{}\n    {} | in={} out={} rates={:?} calibrated={} sync={}",
                    d.id,
                    d.display_name,
                    d.input_channels,
                    d.output_channels,
                    d.sample_rates,
                    d.factory_calibrated,
                    d.synchronous_io
                );
            }
        }
        Cmd::Levels {
            url,
            device,
            sample_rate,
            sine,
            seconds,
            rate_hz,
        } => {
            let client = Client::connect(&url).await?;
            let session = client
                .open_session(
                    &device,
                    pb::DeviceConfig {
                        sample_rate,
                        ..Default::default()
                    },
                )
                .await
                .context("open session")?;
            let generator = match sine {
                None => None,
                Some(s) => {
                    let (f, a) = s.split_once(',').context("--sine expects <hz>,<dbfs>")?;
                    Some(pb::Generator {
                        signal: Some(pb::generator::Signal::Sine(pb::generator::Sine {
                            frequency_hz: f.trim().parse()?,
                            amplitude_dbfs: a.trim().parse()?,
                        })),
                    })
                }
            };
            let mut frames = client.frames();
            let stream = client
                .start_stream(pb::StartStreamRequest {
                    session_id: session.session_id,
                    kind: pb::StreamKind::Levels as i32,
                    block_frames: 0,
                    levels_rate_hz: rate_hz,
                    generator,
                })
                .await
                .context("start stream")?;
            let unit = match stream.scale.and_then(|s| s.unit) {
                Some(pb::scale::Unit::DbvOffset(_)) => "dBV",
                _ => "dBFS",
            };
            let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(seconds);
            while let Ok(Ok(f)) = tokio::time::timeout_at(deadline, frames.recv()).await {
                if f.stream_id != stream.stream_id {
                    continue;
                }
                let cols: Vec<String> = (0..f.channels)
                    .map(|c| {
                        let v = f.channel(c);
                        format!("ch{c} rms {:7.2} peak {:7.2} {unit}", v[0], v[1])
                    })
                    .collect();
                println!("{:>8}  {}", f.first_frame, cols.join("  |  "));
            }
            client.stop_stream(stream.stream_id).await?;
            client.close_session(session.session_id).await?;
        }
    }
    Ok(())
}
