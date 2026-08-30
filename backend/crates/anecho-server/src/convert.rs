//! Mapping between engine/device types and generated contract types.

use anecho_contract::v0 as pb;
use anecho_device::{
    AppliedConfig, BackendKind, Calibration, DeviceConfig, DeviceDescriptor, DeviceError, Scale,
};
use anecho_dsp::{Averaging, Window};
use anecho_engine::generator::{GenLevel, GeneratorSpec, Signal};
use anecho_engine::{
    EngineError, Event, MeasureKind, MeasureRequest, MeasureResult, RtaAxis, RtaConfig,
    ScopeConfig, StreamInfo, StreamKind, StreamRequest, Trigger,
};

pub fn backend_kind(k: BackendKind) -> pb::BackendKind {
    match k {
        BackendKind::Qa40x => pb::BackendKind::Qa40x,
        BackendKind::Cpal => pb::BackendKind::Cpal,
        BackendKind::Virtual => pb::BackendKind::Virtual,
    }
}

pub fn device_info(d: &DeviceDescriptor) -> pb::DeviceInfo {
    let c = &d.capabilities;
    let range = |r: &anecho_device::Range| pb::Range {
        full_scale_dbv: r.full_scale_dbv,
        label: r.label.clone(),
    };
    pb::DeviceInfo {
        id: d.id.to_string(),
        display_name: d.display_name.clone(),
        factory_calibrated: matches!(c.calibration, Calibration::Factory { .. }),
        sample_rates: c.sample_rates.clone(),
        input_channels: c.input_channels as u32,
        output_channels: c.output_channels as u32,
        backend: backend_kind(d.backend) as i32,
        transport: d.transport.clone(),
        input_ranges: c.input_ranges.iter().map(range).collect(),
        output_ranges: c.output_ranges.iter().map(range).collect(),
        synchronous_io: c.synchronous_io,
        nominal_latency_frames: c.nominal_latency_frames,
        firmware_version: d.firmware_version.clone().unwrap_or_default(),
    }
}

pub fn device_config(c: &pb::DeviceConfig) -> Result<DeviceConfig, EngineError> {
    let ch = |v: &[u32]| -> Result<Vec<u16>, EngineError> {
        v.iter()
            .map(|&x| u16::try_from(x).map_err(|_| EngineError::BadRequest(format!("channel {x}"))))
            .collect()
    };
    Ok(DeviceConfig {
        sample_rate: c.sample_rate,
        input_range: c.input_range.map(|x| x as usize),
        output_range: c.output_range.map(|x| x as usize),
        input_channels: ch(&c.input_channels)?,
        output_channels: ch(&c.output_channels)?,
        auto_range_input: c.auto_range_input.unwrap_or(false),
    })
}

pub fn applied_config(a: &AppliedConfig) -> pb::DeviceConfig {
    pb::DeviceConfig {
        sample_rate: a.sample_rate,
        input_range: a.input_range.map(|x| x as u32),
        output_range: a.output_range.map(|x| x as u32),
        input_channels: a.input_channels.iter().map(|&c| c as u32).collect(),
        output_channels: a.output_channels.iter().map(|&c| c as u32).collect(),
        auto_range_input: None,
    }
}

pub fn stream_request(r: &pb::StartStreamRequest) -> Result<StreamRequest, EngineError> {
    let kind = match pb::StreamKind::try_from(r.kind) {
        Ok(pb::StreamKind::Levels) => StreamKind::Levels,
        Ok(pb::StreamKind::RawInput) => StreamKind::RawInput,
        Ok(pb::StreamKind::Rta) => StreamKind::Rta,
        Ok(pb::StreamKind::Scope) => StreamKind::Scope,
        _ => return Err(EngineError::BadRequest("stream kind is required".into())),
    };
    let generator = r.generator.as_ref().map(generator).transpose()?;
    Ok(StreamRequest {
        kind,
        block_frames: r.block_frames,
        levels_rate_hz: r.levels_rate_hz,
        generator,
        rta: r.rta.as_ref().map(rta_config).transpose()?,
        scope: r.scope.as_ref().map(scope_config),
    })
}

pub fn window(w: i32) -> Result<Window, EngineError> {
    use pb::rta_config::Window as W;
    Ok(match W::try_from(w) {
        Ok(W::Unspecified) | Ok(W::Hann) => Window::Hann,
        Ok(W::Rectangular) => Window::Rectangular,
        Ok(W::BlackmanHarris4) => Window::BlackmanHarris4,
        Ok(W::BlackmanHarris7) => Window::BlackmanHarris7,
        Ok(W::FlatTop) => Window::FlatTop,
        Err(_) => return Err(EngineError::BadRequest("unknown window".into())),
    })
}

pub fn rta_config(c: &pb::RtaConfig) -> Result<RtaConfig, EngineError> {
    use pb::rta_config::averaging::Mode;
    let defaults = RtaConfig::default();
    let averaging = match &c.averaging {
        None => Averaging::None,
        Some(a) => {
            let n = if a.count == 0 { 8 } else { a.count };
            match Mode::try_from(a.mode) {
                Ok(Mode::Unspecified) => Averaging::None,
                Ok(Mode::Exponential) => Averaging::Exponential { n },
                Ok(Mode::Linear) => Averaging::Linear { n },
                Ok(Mode::PeakHold) => Averaging::PeakHold,
                Err(_) => return Err(EngineError::BadRequest("unknown averaging mode".into())),
            }
        }
    };
    let min_hz = if c.min_hz > 0.0 {
        c.min_hz as f64
    } else {
        20.0
    };
    let max_hz = if c.max_hz > 0.0 {
        c.max_hz as f64
    } else {
        20_000.0
    };
    let axis = if c.octave_fraction > 0 {
        RtaAxis::Octave {
            fraction: c.octave_fraction,
            min_hz,
            max_hz,
        }
    } else {
        RtaAxis::Log {
            min_hz,
            max_hz,
            points: if c.points == 0 {
                1000
            } else {
                c.points as usize
            },
        }
    };
    Ok(RtaConfig {
        fft_length: if c.fft_length == 0 {
            defaults.fft_length
        } else {
            c.fft_length as usize
        },
        window: window(c.window)?,
        averaging,
        axis,
        update_rate_hz: c.update_rate_hz,
    })
}

pub fn measure_request(r: &pb::MeasureRequest) -> Result<MeasureRequest, EngineError> {
    let kind = match pb::MeasureKind::try_from(r.kind) {
        Ok(pb::MeasureKind::Thd) => MeasureKind::Thd,
        Ok(pb::MeasureKind::ImdSmpte) => MeasureKind::ImdSmpte,
        Ok(pb::MeasureKind::ImdCcif) => MeasureKind::ImdCcif,
        _ => return Err(EngineError::BadRequest("measure kind is required".into())),
    };
    let window = match pb::rta_config::Window::try_from(r.window) {
        Ok(pb::rta_config::Window::Unspecified) => Window::BlackmanHarris7,
        _ => window(r.window)?,
    };
    let band_hz = match (r.band_min_hz > 0.0, r.band_max_hz > 0.0) {
        (false, false) => None,
        (lo, hi) => Some((
            if lo { r.band_min_hz as f64 } else { 20.0 },
            if hi { r.band_max_hz as f64 } else { 20_000.0 },
        )),
    };
    Ok(MeasureRequest {
        kind,
        generator: r.generator.as_ref().map(generator).transpose()?,
        fft_length: r.fft_length as usize,
        window,
        averages: r.averages,
        max_harmonic: r.max_harmonic,
        band_hz,
    })
}

pub fn measure_response(r: &MeasureResult) -> pb::MeasureResponse {
    let kind = match r.kind {
        MeasureKind::Thd => pb::MeasureKind::Thd,
        MeasureKind::ImdSmpte => pb::MeasureKind::ImdSmpte,
        MeasureKind::ImdCcif => pb::MeasureKind::ImdCcif,
    };
    pb::MeasureResponse {
        kind: kind as i32,
        channel: 0,
        per_channel: r
            .per_channel
            .iter()
            .map(|c| pb::DistortionResult {
                fundamental_hz: c.fundamental_hz as f32,
                fundamental_level: c.fundamental_level as f32,
                thd_pct: c.thd_pct as f32,
                thd_db: c.thd_db as f32,
                thd_n_pct: c.thd_n_pct as f32,
                thd_n_db: c.thd_n_db as f32,
                harmonics: c
                    .harmonics
                    .iter()
                    .map(|h| pb::distortion_result::Harmonic {
                        order: h.n,
                        frequency_hz: h.hz as f32,
                        level_db_rel: h.level_db_rel as f32,
                    })
                    .collect(),
                noise_floor_db: c.noise_floor_db as f32,
                imd_pct: c.imd_pct as f32,
                imd_db: c.imd_db as f32,
            })
            .collect(),
        sample_rate: r.sample_rate,
        scale: Some(scale(r.scale)),
    }
}

pub fn scope_config(c: &pb::ScopeConfig) -> ScopeConfig {
    use pb::scope_config::trigger::Mode;
    let trigger = c
        .trigger
        .as_ref()
        .and_then(|t| match Mode::try_from(t.mode) {
            Ok(Mode::Rising) => Some(Trigger {
                rising: true,
                level: t.level,
                channel: t.channel as u16,
            }),
            Ok(Mode::Falling) => Some(Trigger {
                rising: false,
                level: t.level,
                channel: t.channel as u16,
            }),
            _ => None,
        });
    ScopeConfig {
        window_frames: if c.window_frames == 0 {
            4096
        } else {
            c.window_frames as usize
        },
        points: c.points as usize,
        trigger,
    }
}

/// Contract generator → engine spec. `Sine.amplitude_dbfs` keeps its v0.1 meaning when no
/// `level` is given; every other signal needs `level`.
pub fn generator(g: &pb::Generator) -> Result<GeneratorSpec, EngineError> {
    use pb::generator::Signal as S;
    let bad = |m: &str| EngineError::BadRequest(m.to_string());
    let (signal, sine_dbfs) = match &g.signal {
        Some(S::Sine(s)) => (
            Signal::Sine {
                hz: s.frequency_hz as f64,
            },
            Some(s.amplitude_dbfs as f64),
        ),
        Some(S::DualTone(d)) => (
            Signal::DualTone {
                f1: d.f1_hz as f64,
                f2: d.f2_hz as f64,
                ratio_db: d.ratio_db as f64,
            },
            None,
        ),
        Some(S::Multitone(m)) => (
            Signal::Multitone {
                tones: m.frequencies_hz.iter().map(|f| (*f as f64, 0.0)).collect(),
                schroeder: m.schroeder_phases,
            },
            None,
        ),
        Some(S::Noise(n)) => {
            let seed = if n.seed == 0 { 1 } else { n.seed as u64 };
            let kind = pb::generator::NoiseKind::try_from(n.kind)
                .map_err(|_| bad("unknown noise kind"))?;
            let signal = match (kind, n.period_frames) {
                (pb::generator::NoiseKind::White, 0) => Signal::WhiteNoise { seed },
                (pb::generator::NoiseKind::Pink, 0) => Signal::PinkNoise { seed },
                (pb::generator::NoiseKind::White, p) => Signal::PeriodicNoise {
                    seed,
                    period_frames: p as usize,
                },
                (pb::generator::NoiseKind::Pink, _) => {
                    return Err(bad("periodic pink noise is not supported yet"));
                }
                (pb::generator::NoiseKind::Unspecified, _) => {
                    return Err(bad("noise kind is required"));
                }
            };
            (signal, None)
        }
        Some(S::Square(s)) => (
            Signal::Square {
                hz: s.frequency_hz as f64,
            },
            None,
        ),
        None => return Err(bad("generator without signal")),
    };
    let level = match (&g.level, sine_dbfs) {
        (
            Some(pb::generator::Level {
                unit: Some(pb::generator::level::Unit::PeakDbfs(db)),
            }),
            _,
        ) => GenLevel::PeakDbfs(*db as f64),
        (
            Some(pb::generator::Level {
                unit: Some(pb::generator::level::Unit::DbvRms(db)),
            }),
            _,
        ) => GenLevel::DbvRms(*db as f64),
        (_, Some(db)) => GenLevel::PeakDbfs(db),
        (_, None) => return Err(bad("generator level is required for this signal")),
    };
    let output_channels = g
        .output_channels
        .iter()
        .map(|&c| u16::try_from(c).map_err(|_| bad("output channel out of range")))
        .collect::<Result<Vec<u16>, _>>()?;
    Ok(GeneratorSpec {
        signal,
        level,
        output_channels,
    })
}

pub fn event(e: &Event) -> Option<pb::Event> {
    let kind = match e {
        Event::StreamOverrun {
            stream_id,
            dropped_blocks,
        } => pb::event::Kind::StreamOverrun(pb::event::StreamOverrun {
            stream_id: *stream_id,
            dropped_blocks: *dropped_blocks,
        }),
        Event::RangeChanged {
            session_id,
            input_range,
            output_range,
        } => pb::event::Kind::RangeChanged(pb::event::RangeChanged {
            session_id: *session_id,
            input_range: input_range.map(|x| x as u32),
            output_range: output_range.map(|x| x as u32),
        }),
        Event::StreamEnded { .. } => return None,
    };
    Some(pb::Event { kind: Some(kind) })
}

pub fn scale(s: Scale) -> pb::Scale {
    pb::Scale {
        unit: Some(match s {
            Scale::Dbfs => pb::scale::Unit::Dbfs(true),
            Scale::Volts { dbv_offset } => pb::scale::Unit::DbvOffset(dbv_offset),
        }),
    }
}

pub fn stream_started(i: &StreamInfo) -> pb::StartStreamResponse {
    pb::StartStreamResponse {
        stream_id: i.stream_id,
        kind: match i.kind {
            StreamKind::Levels => pb::StreamKind::Levels,
            StreamKind::RawInput => pb::StreamKind::RawInput,
            StreamKind::Rta => pb::StreamKind::Rta,
            StreamKind::Scope => pb::StreamKind::Scope,
        } as i32,
        channels: i.channels as u32,
        sample_rate: i.sample_rate,
        scale: Some(scale(i.scale)),
        values_per_channel: i.values_per_channel as u32,
        axis_hz: i.axis_hz.clone(),
        axis_seconds: i.axis_seconds.clone(),
    }
}

pub fn error_code(e: &EngineError) -> pb::ErrorCode {
    match e {
        EngineError::NoSuchSession(_) | EngineError::NoSuchStream(_) => pb::ErrorCode::NotFound,
        EngineError::StreamRunning(_) => pb::ErrorCode::Busy,
        EngineError::BadRequest(_) => pb::ErrorCode::BadRequest,
        EngineError::Device(d) => match d {
            DeviceError::NotFound(_) | DeviceError::NoSuchStream => pb::ErrorCode::NotFound,
            DeviceError::Busy => pb::ErrorCode::Busy,
            DeviceError::UnsupportedConfig(_) | DeviceError::NotConfigured => {
                pb::ErrorCode::Unsupported
            }
            DeviceError::Disconnected | DeviceError::Backend(_) | DeviceError::Io(_) => {
                pb::ErrorCode::Device
            }
        },
    }
}
