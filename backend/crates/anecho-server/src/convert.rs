//! Mapping between engine/device types and generated contract types.

use anecho_contract::v0 as pb;
use anecho_device::{
    AppliedConfig, BackendKind, Calibration, DeviceConfig, DeviceDescriptor, DeviceError, Scale,
};
use anecho_engine::generator::{GenLevel, GeneratorSpec, Signal};
use anecho_engine::{EngineError, Event, StreamInfo, StreamKind, StreamRequest};

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
        _ => return Err(EngineError::BadRequest("stream kind is required".into())),
    };
    let generator = r.generator.as_ref().map(generator).transpose()?;
    Ok(StreamRequest {
        kind,
        block_frames: r.block_frames,
        levels_rate_hz: r.levels_rate_hz,
        generator,
    })
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
        } as i32,
        channels: i.channels as u32,
        sample_rate: i.sample_rate,
        scale: Some(scale(i.scale)),
        values_per_channel: i.values_per_channel as u32,
        axis_hz: vec![],
        axis_seconds: vec![],
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
