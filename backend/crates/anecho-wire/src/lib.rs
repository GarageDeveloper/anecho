//! Wire helpers shared by server and clients.
//!
//! - [`Frame`]: the real-time binary frame, encoded exactly as documented by
//!   `anecho.v0.BinaryFrame` in `contract/anecho.proto` (little-endian header + f32 values).
//! - Envelope helpers to build requests/responses without repeating boilerplate.

use anecho_contract::Message;
use anecho_contract::v0::{Envelope, envelope::Payload};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use thiserror::Error;

/// Size of the fixed header: u32 + u64 + u64 + u16 + u16.
pub const FRAME_HEADER_LEN: usize = 4 + 8 + 8 + 2 + 2;

/// A decoded real-time frame. `values` is channel-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub stream_id: u32,
    pub seq: u64,
    pub first_frame: u64,
    pub channels: u16,
    pub values_per_channel: u16,
    pub values: Vec<f32>,
}

#[derive(Debug, Error)]
pub enum WireError {
    #[error("frame too short ({0} bytes)")]
    TooShort(usize),
    #[error(
        "frame length mismatch: header announces {expected} values, got {got} bytes of payload"
    )]
    LengthMismatch { expected: usize, got: usize },
    #[error("protobuf decode error: {0}")]
    Decode(#[from] anecho_contract::DecodeError),
}

impl Frame {
    pub fn encode(&self) -> Bytes {
        let mut b = BytesMut::with_capacity(FRAME_HEADER_LEN + self.values.len() * 4);
        b.put_u32_le(self.stream_id);
        b.put_u64_le(self.seq);
        b.put_u64_le(self.first_frame);
        b.put_u16_le(self.channels);
        b.put_u16_le(self.values_per_channel);
        for v in &self.values {
            b.put_f32_le(*v);
        }
        b.freeze()
    }

    pub fn decode(mut bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() < FRAME_HEADER_LEN {
            return Err(WireError::TooShort(bytes.len()));
        }
        let stream_id = bytes.get_u32_le();
        let seq = bytes.get_u64_le();
        let first_frame = bytes.get_u64_le();
        let channels = bytes.get_u16_le();
        let values_per_channel = bytes.get_u16_le();
        let expected = channels as usize * values_per_channel as usize;
        if bytes.len() != expected * 4 {
            return Err(WireError::LengthMismatch {
                expected,
                got: bytes.len(),
            });
        }
        let values = (0..expected).map(|_| bytes.get_f32_le()).collect();
        Ok(Self {
            stream_id,
            seq,
            first_frame,
            channels,
            values_per_channel,
            values,
        })
    }

    /// Values of one channel.
    pub fn channel(&self, ch: u16) -> &[f32] {
        let n = self.values_per_channel as usize;
        &self.values[ch as usize * n..(ch as usize + 1) * n]
    }
}

/// Build an envelope around a payload.
pub fn envelope(request_id: u64, payload: Payload) -> Envelope {
    Envelope {
        request_id,
        payload: Some(payload),
    }
}

pub fn encode_envelope(env: &Envelope) -> Bytes {
    Bytes::from(env.encode_to_vec())
}

pub fn decode_envelope(bytes: &[u8]) -> Result<Envelope, WireError> {
    Ok(Envelope::decode(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip() {
        let f = Frame {
            stream_id: 7,
            seq: 3,
            first_frame: 4096,
            channels: 2,
            values_per_channel: 2,
            values: vec![-20.0, -17.0, -21.0, -18.0],
        };
        let bytes = f.encode();
        assert_eq!(bytes.len(), FRAME_HEADER_LEN + 16);
        assert_eq!(Frame::decode(&bytes).unwrap(), f);
        assert_eq!(f.channel(1), &[-21.0, -18.0]);
    }

    #[test]
    fn frame_rejects_bad_length() {
        let mut bytes = Frame {
            stream_id: 1,
            seq: 0,
            first_frame: 0,
            channels: 1,
            values_per_channel: 3,
            values: vec![0.0; 3],
        }
        .encode()
        .to_vec();
        bytes.pop();
        assert!(matches!(
            Frame::decode(&bytes),
            Err(WireError::LengthMismatch { .. })
        ));
        assert!(matches!(
            Frame::decode(&bytes[..5]),
            Err(WireError::TooShort(5))
        ));
    }
}
