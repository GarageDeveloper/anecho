//! Oscilloscope: a window of samples, optionally aligned on a trigger, decimated for display.
//!
//! Values are raw samples in full-scale units (±1.0), not dB. When `points < window_frames`
//! every k-th sample is kept (`k = ceil(window / points)`) — plain sub-sampling, no min/max
//! envelope, so fast transients between kept samples are simply not shown; the time axis
//! (`axis_seconds`) tells exactly which instants are.
//!
//! Triggering searches the first half of a two-window buffer for a level crossing on the
//! trigger channel (rising: `x[i-1] < level <= x[i]`); the displayed window starts there.
//! Without a crossing (or free running) it starts at the buffer head.

use super::Reading;
use anecho_device::InputBlock;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trigger {
    pub rising: bool,
    pub level: f32,
    pub channel: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeConfig {
    pub window_frames: usize,
    /// Points sent per frame; 0 = `window_frames`.
    pub points: usize,
    pub trigger: Option<Trigger>,
}

#[derive(Debug)]
pub struct Scope {
    channels: usize,
    window: usize,
    step: usize,
    points: usize,
    trigger: Option<Trigger>,
    /// Interleaved samples, at most 2 windows.
    buf: Vec<f32>,
    buf_first_frame: u64,
    sample_rate: u32,
}

impl Scope {
    pub fn new(cfg: &ScopeConfig, channels: u16, sample_rate: u32) -> Self {
        let window = cfg.window_frames.max(2);
        let points = if cfg.points == 0 {
            window
        } else {
            cfg.points.clamp(1, window)
        };
        let step = window.div_ceil(points);
        let points = window.div_ceil(step);
        Self {
            channels: channels as usize,
            window,
            step,
            points,
            trigger: cfg.trigger,
            buf: Vec::with_capacity(2 * window * channels as usize),
            buf_first_frame: 0,
            sample_rate,
        }
    }

    pub fn points(&self) -> usize {
        self.points
    }

    /// Time of each displayed point from the window start, in seconds.
    pub fn axis_seconds(&self) -> Vec<f32> {
        (0..self.points)
            .map(|i| (i * self.step) as f32 / self.sample_rate as f32)
            .collect()
    }

    pub fn push(&mut self, block: &InputBlock) -> Vec<Reading> {
        let ch = self.channels;
        if self.buf.is_empty() {
            self.buf_first_frame = block.first_frame;
        }
        self.buf.extend_from_slice(&block.samples);
        let mut out = Vec::new();
        while self.buf.len() / ch >= 2 * self.window {
            let start = self.find_trigger().unwrap_or(0);
            let mut values = Vec::with_capacity(ch * self.points);
            for c in 0..ch {
                for p in 0..self.points {
                    values.push(self.buf[(start + p * self.step) * ch + c]);
                }
            }
            out.push(Reading {
                first_frame: self.buf_first_frame + start as u64,
                values,
            });
            // Drop one window; keep the rest for the next search.
            self.buf.drain(..self.window * ch);
            self.buf_first_frame += self.window as u64;
        }
        out
    }

    fn find_trigger(&self) -> Option<usize> {
        let t = self.trigger?;
        let ch = self.channels;
        let c = (t.channel as usize).min(ch - 1);
        (1..self.window).find(|&i| {
            let prev = self.buf[(i - 1) * ch + c];
            let cur = self.buf[i * ch + c];
            if t.rising {
                prev < t.level && cur >= t.level
            } else {
                prev > t.level && cur <= t.level
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn rising_trigger_aligns_the_window() {
        let fs = 48_000;
        // 1 kHz mono sine, starting at phase π/2 so the first rising zero crossing is later.
        let samples: Vec<f32> = (0..2048)
            .map(|i| {
                (std::f64::consts::TAU * 1000.0 * i as f64 / fs as f64
                    + std::f64::consts::FRAC_PI_2)
                    .sin() as f32
            })
            .collect();
        let block = InputBlock {
            seq: 0,
            first_frame: 0,
            channels: 1,
            frames: 2048,
            samples: Arc::from(samples),
            dropped_before: 0,
            scale: anecho_device::Scale::Dbfs,
        };
        let cfg = ScopeConfig {
            window_frames: 480,
            points: 48,
            trigger: Some(Trigger {
                rising: true,
                level: 0.0,
                channel: 0,
            }),
        };
        let mut scope = Scope::new(&cfg, 1, fs);
        let readings = scope.push(&block);
        assert!(!readings.is_empty());
        let r = &readings[0];
        assert_eq!(r.values.len(), 48);
        // First rising zero crossing of cos is at 3/4 period = 36 samples (the sample there
        // is ±1e-16, so the crossing is detected at 36 or 37).
        assert!((36..=37).contains(&r.first_frame), "{}", r.first_frame);
        assert!(r.values[0].abs() < 0.15 && r.values[1] > r.values[0]);
        assert_eq!(scope.axis_seconds()[1], 10.0 / fs as f32);
    }
}
