//! Anecho API contract — Rust side.
//!
//! Every type here is generated from `contract/*.proto` by `build.rs`. Do not add
//! hand-written message types to this crate; extend the `.proto` files instead
//! (additively, in a separate commit — see CLAUDE.md).

#![allow(missing_debug_implementations)]

/// Contract version `anecho.v0`.
pub mod v0 {
    include!(concat!(env!("OUT_DIR"), "/anecho.v0.rs"));
}

pub use prost::Message;

#[cfg(test)]
mod tests {
    use super::v0::{DeviceInfo, Envelope, ListDevicesResponse, envelope::Payload};
    use prost::Message;

    #[test]
    fn envelope_round_trips() {
        let env = Envelope {
            request_id: 42,
            payload: Some(Payload::Devices(ListDevicesResponse {
                devices: vec![DeviceInfo {
                    id: "qa40x/demo".into(),
                    display_name: "QA403 (demo)".into(),
                    factory_calibrated: true,
                    sample_rates: vec![48_000, 96_000, 192_000, 384_000],
                    input_channels: 2,
                    output_channels: 2,
                    ..Default::default()
                }],
            })),
        };
        let bytes = env.encode_to_vec();
        let back = Envelope::decode(bytes.as_slice()).unwrap();
        assert_eq!(back, env);
    }
}
