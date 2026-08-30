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
        /// Do not expose QA40x units on the USB bus.
        #[arg(long)]
        no_qa40x: bool,
        /// Also expose the embedded QA40x simulator (build feature qa40x-sim).
        #[arg(long)]
        qa40x_sim: bool,
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
        /// Input range index (see `devices`); default: the device's safe default.
        #[arg(long)]
        input_range: Option<u32>,
        /// Drive the outputs with a sine: `<frequency_hz>,<amplitude_dbfs>` (alias of
        /// `--signal sine:<hz> --level <dbfs>dBFS`).
        #[arg(long)]
        sine: Option<String>,
        /// Drive the outputs: `sine:1000`, `dual:60,7000[,12]`, `multi:100,1000,10000`,
        /// `white`, `pink`, `periodic[:frames]`, `square:1000`.
        #[arg(long)]
        signal: Option<String>,
        /// Generator level: `-20dBFS` (peak) or `-10dBV` (RMS, calibrated devices only).
        #[arg(long)]
        level: Option<String>,
        #[arg(long, default_value_t = 2.0)]
        seconds: f64,
        #[arg(long, default_value_t = 10.0)]
        rate_hz: f32,
    },
    /// Real-time spectrum analyzer: print the last frame as a compact table.
    Rta {
        #[arg(long, default_value = "ws://127.0.0.1:4800/ws")]
        url: String,
        #[arg(long)]
        device: String,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Input range index (see `devices`); default: the device's safe default.
        #[arg(long)]
        input_range: Option<u32>,
        #[arg(long, default_value_t = 16_384)]
        fft: u32,
        /// rect, hann, bh4, bh7, flattop.
        #[arg(long, default_value = "hann")]
        window: String,
        /// none, exp[:n], linear[:n], peak.
        #[arg(long, default_value = "none")]
        averaging: String,
        /// Log-axis points between --min-hz and --max-hz (ignored with --octave).
        #[arg(long, default_value_t = 200)]
        points: u32,
        /// Fractional-octave display: 1 = 1/1, 3 = 1/3, ...
        #[arg(long)]
        octave: Option<u32>,
        #[arg(long, default_value_t = 20.0)]
        min_hz: f32,
        #[arg(long, default_value_t = 20_000.0)]
        max_hz: f32,
        #[arg(long)]
        signal: Option<String>,
        #[arg(long)]
        level: Option<String>,
        #[arg(long, default_value_t = 2.0)]
        seconds: f64,
    },
    /// Oscilloscope: print the last window as a table.
    Scope {
        #[arg(long, default_value = "ws://127.0.0.1:4800/ws")]
        url: String,
        #[arg(long)]
        device: String,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// Input range index (see `devices`); default: the device's safe default.
        #[arg(long)]
        input_range: Option<u32>,
        #[arg(long, default_value_t = 480)]
        window_frames: u32,
        #[arg(long, default_value_t = 48)]
        points: u32,
        /// rising or falling (free running when absent).
        #[arg(long)]
        trigger: Option<String>,
        #[arg(long, default_value_t = 0.0)]
        trigger_level: f32,
        #[arg(long)]
        signal: Option<String>,
        #[arg(long)]
        level: Option<String>,
        #[arg(long, default_value_t = 1.0)]
        seconds: f64,
    },
    /// One-shot distortion measurement: thd, imd-smpte or imd-ccif.
    Measure {
        #[arg(long, default_value = "ws://127.0.0.1:4800/ws")]
        url: String,
        #[arg(long)]
        device: String,
        #[arg(long, default_value_t = 48_000)]
        sample_rate: u32,
        /// thd, imd-smpte, imd-ccif.
        #[arg(long, default_value = "thd")]
        kind: String,
        /// Generator signal; defaults to the kind's standard stimulus.
        #[arg(long)]
        signal: Option<String>,
        #[arg(long, default_value = "-20dBFS")]
        level: String,
        #[arg(long, default_value_t = 65_536)]
        fft: u32,
        #[arg(long, default_value_t = 4)]
        averages: u32,
        /// Input range index (see `devices`); default: the device's safe default.
        #[arg(long)]
        input_range: Option<u32>,
    },
}

mod cli;

/// Open a session and collect frames of one stream for `seconds`; returns the last frame.
async fn run_stream(
    url: &str,
    device: &str,
    sample_rate: u32,
    input_range: Option<u32>,
    req: pb::StartStreamRequest,
    seconds: f64,
) -> anyhow::Result<(pb::StartStreamResponse, Option<anecho_wire::Frame>, usize)> {
    let client = Client::connect(url).await?;
    let session = client
        .open_session(
            device,
            pb::DeviceConfig {
                sample_rate,
                input_range,
                ..Default::default()
            },
        )
        .await
        .context("open session")?;
    let mut frames = client.frames();
    let stream = client
        .start_stream(pb::StartStreamRequest {
            session_id: session.session_id,
            ..req
        })
        .await
        .context("start stream")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(seconds);
    let mut last = None;
    let mut count = 0usize;
    while let Ok(Ok(f)) = tokio::time::timeout_at(deadline, frames.recv()).await {
        if f.stream_id == stream.stream_id {
            last = Some(f);
            count += 1;
        }
    }
    client.stop_stream(stream.stream_id).await?;
    client.close_session(session.session_id).await?;
    Ok((stream, last, count))
}

fn unit_of(scale: Option<&pb::Scale>) -> &'static str {
    match scale.and_then(|s| s.unit.as_ref()) {
        Some(pb::scale::Unit::DbvOffset(_)) => "dBV",
        _ => "dBFS",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    match Cli::parse().cmd {
        Cmd::Serve {
            bind,
            virtual_loopback,
            no_cpal,
            no_qa40x,
            qa40x_sim,
        } => {
            let engine = anecho::build_engine(&anecho::BackendOptions {
                virtual_loopback,
                cpal: !no_cpal,
                qa40x: !no_qa40x,
                qa40x_sim,
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
                    "{}\n    {} | in={} out={} rates={:?} calibrated={} sync={}{}",
                    d.id,
                    d.display_name,
                    d.input_channels,
                    d.output_channels,
                    d.sample_rates,
                    d.factory_calibrated,
                    d.synchronous_io,
                    if d.firmware_version.is_empty() {
                        String::new()
                    } else {
                        format!(" firmware={}", d.firmware_version)
                    }
                );
            }
        }
        Cmd::Levels {
            url,
            device,
            sample_rate,
            input_range,
            sine,
            signal,
            level,
            seconds,
            rate_hz,
        } => {
            let client = Client::connect(&url).await?;
            let session = client
                .open_session(
                    &device,
                    pb::DeviceConfig {
                        sample_rate,
                        input_range,
                        ..Default::default()
                    },
                )
                .await
                .context("open session")?;
            let generator = cli::generator(signal.as_deref(), level.as_deref(), sine.as_deref())?;
            let mut frames = client.frames();
            let stream = client
                .start_stream(pb::StartStreamRequest {
                    session_id: session.session_id,
                    kind: pb::StreamKind::Levels as i32,
                    block_frames: 0,
                    levels_rate_hz: rate_hz,
                    generator,
                    ..Default::default()
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
        Cmd::Rta {
            url,
            device,
            sample_rate,
            input_range,
            fft,
            window,
            averaging,
            points,
            octave,
            min_hz,
            max_hz,
            signal,
            level,
            seconds,
        } => {
            let generator = cli::generator(signal.as_deref(), level.as_deref(), None)?;
            let tone_hz = cli::generator_hz(generator.as_ref());
            let req = pb::StartStreamRequest {
                kind: pb::StreamKind::Rta as i32,
                generator,
                rta: Some(pb::RtaConfig {
                    fft_length: fft,
                    window: cli::parse_window(&window)? as i32,
                    averaging: Some(cli::parse_averaging(&averaging)?),
                    min_hz,
                    max_hz,
                    points,
                    octave_fraction: octave.unwrap_or(0),
                    update_rate_hz: 0.0,
                }),
                ..Default::default()
            };
            let (stream, last, count) =
                run_stream(&url, &device, sample_rate, input_range, req, seconds).await?;
            let unit = unit_of(stream.scale.as_ref());
            let Some(f) = last else {
                anyhow::bail!("no RTA frame received in {seconds} s");
            };
            println!(
                "{count} frames, {} points, {} channels, values in {unit} (RMS)",
                stream.axis_hz.len(),
                f.channels
            );
            for c in 0..f.channels {
                let v = f.channel(c);
                let mut idx: Vec<usize> = (0..v.len()).collect();
                idx.sort_by(|a, b| v[*b].partial_cmp(&v[*a]).unwrap());
                println!("ch{c} top 10:");
                for i in idx.iter().take(10) {
                    println!("    {:9.1} Hz  {:7.2} {unit}", stream.axis_hz[*i], v[*i]);
                }
                if let Some(hz) = tone_hz {
                    let (i, _) = stream
                        .axis_hz
                        .iter()
                        .enumerate()
                        .min_by(|a, b| (a.1 - hz).abs().partial_cmp(&(b.1 - hz).abs()).unwrap())
                        .unwrap();
                    println!(
                        "    at {hz:.1} Hz (point {:.1} Hz): {:7.2} {unit}",
                        stream.axis_hz[i], v[i]
                    );
                }
            }
        }
        Cmd::Scope {
            url,
            device,
            sample_rate,
            input_range,
            window_frames,
            points,
            trigger,
            trigger_level,
            signal,
            level,
            seconds,
        } => {
            let generator = cli::generator(signal.as_deref(), level.as_deref(), None)?;
            let mode = match trigger.as_deref() {
                None => pb::scope_config::trigger::Mode::Unspecified,
                Some("rising") => pb::scope_config::trigger::Mode::Rising,
                Some("falling") => pb::scope_config::trigger::Mode::Falling,
                Some(o) => anyhow::bail!("unknown trigger {o:?} (rising, falling)"),
            };
            let req = pb::StartStreamRequest {
                kind: pb::StreamKind::Scope as i32,
                generator,
                scope: Some(pb::ScopeConfig {
                    window_frames,
                    points,
                    trigger: Some(pb::scope_config::Trigger {
                        mode: mode as i32,
                        level: trigger_level,
                        channel: 0,
                    }),
                }),
                ..Default::default()
            };
            let (stream, last, count) =
                run_stream(&url, &device, sample_rate, input_range, req, seconds).await?;
            let Some(f) = last else {
                anyhow::bail!("no scope frame received in {seconds} s");
            };
            println!(
                "{count} windows, {} points, window starts at frame {}",
                stream.axis_seconds.len(),
                f.first_frame
            );
            println!(
                "{:>9}  {}",
                "t (ms)",
                (0..f.channels)
                    .map(|c| format!("{:>9}", format!("ch{c}")))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            for (i, t) in stream.axis_seconds.iter().enumerate() {
                let cols: Vec<String> = (0..f.channels)
                    .map(|c| format!("{:9.4}", f.channel(c)[i]))
                    .collect();
                println!("{:9.3}  {}", t * 1000.0, cols.join(" "));
            }
        }
        Cmd::Measure {
            url,
            device,
            sample_rate,
            kind,
            signal,
            level,
            fft,
            averages,
            input_range,
        } => {
            let (kind, default_signal) = match kind.as_str() {
                "thd" => (pb::MeasureKind::Thd, "sine:1000"),
                "imd-smpte" => (pb::MeasureKind::ImdSmpte, "dual:60,7000,12.04"),
                "imd-ccif" => (pb::MeasureKind::ImdCcif, "dual:19000,20000,0"),
                other => anyhow::bail!("unknown kind {other:?} (thd, imd-smpte, imd-ccif)"),
            };
            let generator = cli::generator(
                Some(signal.as_deref().unwrap_or(default_signal)),
                Some(&level),
                None,
            )?;
            let client = Client::connect(&url).await?;
            let session = client
                .open_session(
                    &device,
                    pb::DeviceConfig {
                        sample_rate,
                        input_range,
                        ..Default::default()
                    },
                )
                .await
                .context("open session")?;
            if let Some(dev) = session
                .device
                .as_ref()
                .filter(|d| !d.firmware_version.is_empty())
            {
                println!("{} firmware {}", dev.display_name, dev.firmware_version);
            }
            let m = client
                .measure(pb::MeasureRequest {
                    session_id: session.session_id,
                    kind: kind as i32,
                    generator,
                    fft_length: fft,
                    averages,
                    ..Default::default()
                })
                .await
                .context("measure")?;
            client.close_session(session.session_id).await?;
            let unit = unit_of(m.scale.as_ref());
            println!(
                "{} at {} Hz, FFT {fft}, {averages} averages",
                kind.as_str_name(),
                m.sample_rate
            );
            for (c, r) in m.per_channel.iter().enumerate() {
                println!(
                    "ch{c}: fundamental {:.2} Hz at {:.2} {unit} (RMS)",
                    r.fundamental_hz, r.fundamental_level
                );
                match kind {
                    pb::MeasureKind::Thd => {
                        println!(
                            "     THD {:.5} % ({:.2} dB)   THD+N {:.5} % ({:.2} dB)   noise floor {:.1} dB/bin",
                            r.thd_pct, r.thd_db, r.thd_n_pct, r.thd_n_db, r.noise_floor_db
                        );
                        for h in &r.harmonics {
                            println!(
                                "     H{:<2} {:9.1} Hz  {:7.2} dBc",
                                h.order, h.frequency_hz, h.level_db_rel
                            );
                        }
                    }
                    _ => println!("     IMD {:.5} % ({:.2} dB)", r.imd_pct, r.imd_db),
                }
            }
        }
    }
    Ok(())
}
