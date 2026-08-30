//! Input auto-range: pick the input range from the signal while streaming.
//!
//! Policy (all timings in audio frames, so it behaves the same on a non-realtime
//! simulator):
//! - **up** one range as soon as the peak of the last 3 blocks exceeds −1 dBFS (clipping);
//! - **down** one range when that peak has stayed below −(step + 12) dB for one second
//!   (a 6 dB step means: below −18 dBFS, the next range down still leaves 12 dB headroom);
//! - never more than one change per 300 ms; hysteresis comes from the asymmetric thresholds.
//!
//! The range write happens between two capture blocks through
//! [`MeasurementDevice::set_input_range`]; the caller then refreshes its dB offset from
//! `device.scale(Direction::Input)`.

use anecho_device::{InputBlock, MeasurementDevice};
use std::collections::VecDeque;
use std::sync::Arc;

pub const CLIP_DBFS: f32 = -1.0;
pub const DOWN_MARGIN_DB: f32 = 12.0;
pub const DOWN_HOLD_SECONDS: f64 = 1.0;
pub const MIN_INTERVAL_SECONDS: f64 = 0.3;

pub struct AutoRange {
    device: Arc<dyn MeasurementDevice>,
    current: usize,
    n_ranges: usize,
    step_db: f32,
    peaks: VecDeque<f32>,
    low_since: Option<u64>,
    last_change: Option<u64>,
    hold_frames: u64,
    interval_frames: u64,
}

impl std::fmt::Debug for AutoRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoRange")
            .field("current", &self.current)
            .field("n_ranges", &self.n_ranges)
            .finish_non_exhaustive()
    }
}

impl AutoRange {
    /// `None` when the device has fewer than two input ranges.
    pub fn new(
        device: Arc<dyn MeasurementDevice>,
        current: usize,
        sample_rate: u32,
    ) -> Option<Self> {
        let ranges = &device.capabilities().input_ranges;
        if ranges.len() < 2 {
            return None;
        }
        let step_db = (ranges[1].full_scale_dbv - ranges[0].full_scale_dbv)
            .abs()
            .max(1.0);
        Some(Self {
            n_ranges: ranges.len(),
            current: current.min(ranges.len() - 1),
            step_db,
            peaks: VecDeque::with_capacity(3),
            low_since: None,
            last_change: None,
            hold_frames: (DOWN_HOLD_SECONDS * sample_rate as f64) as u64,
            interval_frames: (MIN_INTERVAL_SECONDS * sample_rate as f64) as u64,
            device,
        })
    }

    pub fn current(&self) -> usize {
        self.current
    }

    /// Look at a block; when a range change is due, apply it and return the new index.
    pub async fn observe(&mut self, block: &InputBlock) -> Option<usize> {
        let peak = block.samples.iter().fold(0f32, |m, v| m.max(v.abs()));
        let peak_db = if peak > 0.0 {
            20.0 * peak.log10()
        } else {
            -200.0
        };
        if self.peaks.len() == 3 {
            self.peaks.pop_front();
        }
        self.peaks.push_back(peak_db);
        let recent = self.peaks.iter().cloned().fold(f32::MIN, f32::max);
        let now = block.first_frame;
        let interval_ok = self
            .last_change
            .is_none_or(|t| now.saturating_sub(t) >= self.interval_frames);

        let target = if recent > CLIP_DBFS {
            self.low_since = None;
            (self.current + 1 < self.n_ranges && interval_ok).then(|| self.current + 1)
        } else if recent < -(self.step_db + DOWN_MARGIN_DB) {
            let since = *self.low_since.get_or_insert(now);
            (now.saturating_sub(since) >= self.hold_frames && self.current > 0 && interval_ok)
                .then(|| self.current - 1)
        } else {
            self.low_since = None;
            None
        };
        let target = target?;
        match self.device.set_input_range(target).await {
            Ok(()) => {
                self.current = target;
                self.last_change = Some(now);
                self.low_since = None;
                self.peaks.clear();
                Some(target)
            }
            Err(e) => {
                log::warn!("auto-range: cannot switch to input range {target}: {e}");
                self.last_change = Some(now);
                None
            }
        }
    }
}
