//! Mapping between engine/device types and generated contract types.

use anecho_contract::v0 as pb;
use anecho_device::{
    AppliedConfig, BackendKind, Calibration, DeviceConfig, DeviceDescriptor, DeviceError, Scale,
};
use anecho_engine::{EngineError, StreamInfo, StreamKind, StreamRequest, generator::Signal};

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
    })
}

pub fn applied_config(a: &AppliedConfig) -> pb::DeviceConfig {
    pb::DeviceConfig {
        sample_rate: a.sample_rate,
        input_range: a.input_range.map(|x| x as u32),
        output_range: a.output_range.map(|x| x as u32),
        input_channels: a.input_channels.iter().map(|&c| c as u32).collect(),
        output_channels: a.output_channels.iter().map(|&c| c as u32).collect(),
    }
}

pub fn stream_request(r: &pb::StartStreamRequest) -> Result<StreamRequest, EngineError> {
    let kind = match pb::StreamKind::try_from(r.kind) {
        Ok(pb::StreamKind::Levels) => StreamKind::Levels,
        Ok(pb::StreamKind::RawInput) => StreamKind::RawInput,
        _ => return Err(EngineError::BadRequest("stream kind is required".into())),
    };
    let generator = match &r.generator {
        None => None,
        Some(pb::Generator {
            signal: Some(pb::generator::Signal::Sine(s)),
        }) => Some(Signal::Sine {
            frequency_hz: s.frequency_hz,
            amplitude_dbfs: s.amplitude_dbfs,
        }),
        Some(_) => return Err(EngineError::BadRequest("generator without signal".into())),
    };
    Ok(StreamRequest {
        kind,
        block_frames: r.block_frames,
        levels_rate_hz: r.levels_rate_hz,
        generator,
    })
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
