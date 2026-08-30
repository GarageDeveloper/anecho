//! Level meter: per-channel RMS and peak in dBFS, decimated to a display rate.

use anecho_device::InputBlock;

#[derive(Debug)]
pub struct LevelMeter {
    channels: usize,
    window_frames: usize,
    acc_sq: Vec<f64>,
    peak: Vec<f32>,
    frames_in_window: usize,
    window_first_frame: u64,
}

/// One meter reading: `values` is channel-major `[rms_db, peak_db]` per channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub first_frame: u64,
    pub values: Vec<f32>,
}

/// Floor used for silence so that logs never yield -inf.
pub const SILENCE_DB: f32 = -200.0;

pub fn to_db(x: f64) -> f32 {
    if x <= 0.0 {
        SILENCE_DB
    } else {
        (20.0 * x.log10()).max(SILENCE_DB as f64) as f32
    }
}

impl LevelMeter {
    pub fn new(channels: u16, sample_rate: u32, rate_hz: f32) -> Self {
        let window_frames = ((sample_rate as f32 / rate_hz).round() as usize).max(1);
        Self {
            channels: channels as usize,
            window_frames,
            acc_sq: vec![0.0; channels as usize],
            peak: vec![0.0; channels as usize],
            frames_in_window: 0,
            window_first_frame: 0,
        }
    }

    pub fn window_frames(&self) -> usize {
        self.window_frames
    }

    /// Feed a block; returns zero or more completed readings.
    pub fn push(&mut self, block: &InputBlock) -> Vec<Reading> {
        let ch = self.channels;
        let mut out = Vec::new();
        for (i, frame) in block.samples.chunks_exact(ch).enumerate() {
            if self.frames_in_window == 0 {
                self.window_first_frame = block.first_frame + i as u64;
            }
            for (c, &v) in frame.iter().enumerate() {
                self.acc_sq[c] += (v as f64) * (v as f64);
                let a = v.abs();
                if a > self.peak[c] {
                    self.peak[c] = a;
                }
            }
            self.frames_in_window += 1;
            if self.frames_in_window == self.window_frames {
                out.push(self.flush());
            }
        }
        out
    }

    fn flush(&mut self) -> Reading {
        let n = self.frames_in_window as f64;
        let mut values = Vec::with_capacity(self.channels * 2);
        for c in 0..self.channels {
            values.push(to_db((self.acc_sq[c] / n).sqrt()));
            values.push(to_db(self.peak[c] as f64));
            self.acc_sq[c] = 0.0;
            self.peak[c] = 0.0;
        }
        self.frames_in_window = 0;
        Reading {
            first_frame: self.window_first_frame,
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn full_scale_sine_reads_minus_3_db_rms_and_0_db_peak() {
        let sr = 48_000;
        let mut m = LevelMeter::new(1, sr, 10.0); // 4800-frame windows
        let samples: Vec<f32> = (0..9600)
            .map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / sr as f32).sin())
            .collect();
        let block = InputBlock {
            seq: 0,
            first_frame: 0,
            channels: 1,
            frames: 9600,
            samples: Arc::from(samples),
            dropped_before: 0,
        };
        let readings = m.push(&block);
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[1].first_frame, 4800);
        let r = &readings[0].values;
        assert!((r[0] + 3.01).abs() < 0.02, "rms {}", r[0]);
        assert!(r[1].abs() < 0.01, "peak {}", r[1]);
    }

    #[test]
    fn silence_is_floored() {
        assert_eq!(to_db(0.0), SILENCE_DB);
    }
}
