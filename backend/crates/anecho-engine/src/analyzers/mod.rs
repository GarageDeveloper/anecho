//! Stream analyzers: turn captured [`anecho_device::InputBlock`]s into ready-to-plot frames.

pub mod rta;
pub mod scope;

/// One emitted frame: channel-major values and the index of the first audio frame covered.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub first_frame: u64,
    pub values: Vec<f32>,
}
